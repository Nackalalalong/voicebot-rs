use async_trait::async_trait;
use common::error::AsrError;
use common::events::PipelineEvent;
use common::traits::{AsrProvider, AudioInputStream};
use futures::StreamExt;
use reqwest::multipart;
use tokio::sync::mpsc::Sender;
use tracing::debug;

/// Speaches ASR provider using the `/v1/audio/transcriptions` endpoint.
/// Supports SSE streaming mode (`stream: true`) for partial transcripts.
pub struct SpeachesAsrProvider {
    client: reqwest::Client,
    pub(crate) base_url: String,
    pub(crate) api_key: Option<String>,
    pub(crate) model: String,
    pub(crate) language: Option<String>,
}

impl SpeachesAsrProvider {
    pub fn new(base_url: String, model: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            api_key: None,
            model,
            language: None,
        }
    }

    pub fn with_api_key(mut self, api_key: String) -> Self {
        self.api_key = Some(api_key);
        self
    }

    pub fn with_language(mut self, language: String) -> Self {
        self.language = Some(language);
        self
    }

    /// Wrap raw PCM i16 LE samples in a WAV container (16kHz mono 16-bit).
    fn wrap_wav(pcm_bytes: &[u8]) -> Vec<u8> {
        let data_len = pcm_bytes.len() as u32;
        let file_len = 36 + data_len;
        let mut buf = Vec::with_capacity(44 + pcm_bytes.len());
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&file_len.to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes()); // chunk size
        buf.extend_from_slice(&1u16.to_le_bytes()); // PCM format
        buf.extend_from_slice(&1u16.to_le_bytes()); // mono
        buf.extend_from_slice(&16000u32.to_le_bytes()); // sample rate
        buf.extend_from_slice(&32000u32.to_le_bytes()); // byte rate (16000 * 1 * 2)
        buf.extend_from_slice(&2u16.to_le_bytes()); // block align
        buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_len.to_le_bytes());
        buf.extend_from_slice(pcm_bytes);
        buf
    }

    /// Build the multipart form from collected PCM bytes.
    fn build_form(&self, pcm_bytes: Vec<u8>, streaming: bool) -> Result<multipart::Form, AsrError> {
        let wav_bytes = Self::wrap_wav(&pcm_bytes);
        let file_part = multipart::Part::bytes(wav_bytes)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| AsrError::InvalidResponse(format!("mime error: {e}")))?;

        let mut form = multipart::Form::new()
            .part("file", file_part)
            .text("model", self.model.clone())
            .text("response_format", "verbose_json");

        if streaming {
            form = form.text("stream", "true");
        }

        if let Some(lang) = &self.language {
            form = form.text("language", lang.clone());
        }

        Ok(form)
    }

    /// Send the request and handle HTTP-level errors.
    async fn send_request(&self, form: multipart::Form) -> Result<reqwest::Response, AsrError> {
        let mut req = self
            .client
            .post(format!("{}/v1/audio/transcriptions", self.base_url))
            .multipart(form)
            .timeout(std::time::Duration::from_secs(30));

        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = req.send().await.map_err(|e| {
            if e.is_timeout() {
                AsrError::Timeout(30000)
            } else {
                AsrError::ConnectionFailed
            }
        })?;

        if !resp.status().is_success() {
            return Err(AsrError::ProviderUnavailable(format!(
                "Speaches ASR returned {}",
                resp.status()
            )));
        }

        Ok(resp)
    }
}

/// SSE segment from streaming transcription (verbose_json format).
#[derive(serde::Deserialize)]
struct SseSegment {
    text: String,
}

#[async_trait]
impl AsrProvider for SpeachesAsrProvider {
    async fn stream(
        &self,
        mut audio: Box<dyn AudioInputStream>,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), AsrError> {
        // Collect all audio frames until the stream ends
        let mut pcm_bytes: Vec<u8> = Vec::new();
        while let Some(frame) = audio.recv().await {
            pcm_bytes.extend_from_slice(&frame.to_pcm_bytes());
        }
        tracing::debug!(
            bytes = pcm_bytes.len(),
            duration_ms = pcm_bytes.len() / 32,
            "ASR audio collected"
        );

        if pcm_bytes.is_empty() {
            tracing::debug!("ASR skipping empty audio");
            return Ok(());
        }

        // Use SSE streaming to get partial transcripts
        let form = self.build_form(pcm_bytes, true)?;
        tracing::debug!(url = %format!("{}/v1/audio/transcriptions", self.base_url), model = %self.model, language = ?self.language, "ASR sending request");
        let resp = self.send_request(form).await?;
        tracing::debug!("ASR request accepted, reading SSE stream");

        let language = self.language.clone().unwrap_or_else(|| "auto".into());
        let mut full_text = String::new();
        let mut byte_stream = resp.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = byte_stream.next().await {
            let bytes = chunk
                .map_err(|e| AsrError::InvalidResponse(format!("SSE stream read error: {e}")))?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            // Process complete SSE lines
            while let Some(newline_pos) = buffer.find('\n') {
                let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
                buffer.drain(..=newline_pos);

                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        debug!("ASR SSE stream done");
                        continue;
                    }
                    if let Ok(segment) = serde_json::from_str::<SseSegment>(data) {
                        if !segment.text.is_empty() {
                            tracing::debug!(text = %segment.text, "ASR partial transcript");
                            tx.send(PipelineEvent::PartialTranscript {
                                text: segment.text.clone(),
                                confidence: 0.0,
                            })
                            .await
                            .map_err(|_| AsrError::ChannelClosed)?;
                            full_text.push_str(&segment.text);
                        }
                    }
                }
            }
        }

        // Emit the final aggregated transcript
        if !full_text.is_empty() {
            debug!(text = %full_text, "ASR final transcript");
            tx.send(PipelineEvent::FinalTranscript {
                text: full_text,
                language,
            })
            .await
            .map_err(|_| AsrError::ChannelClosed)?;
        } else {
            debug!("ASR produced no transcript");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse_segment_parse() {
        let json = r#"{"text": "hello world"}"#;
        let segment: SseSegment = serde_json::from_str(json).unwrap();
        assert_eq!(segment.text, "hello world");
    }

    #[test]
    fn test_provider_builder() {
        let provider = SpeachesAsrProvider::new(
            "http://localhost:8000".into(),
            "Systran/faster-distil-whisper-large-v3".into(),
        )
        .with_api_key("test-key".into())
        .with_language("en".into());

        assert_eq!(provider.base_url, "http://localhost:8000");
        assert_eq!(provider.model, "Systran/faster-distil-whisper-large-v3");
        assert_eq!(provider.api_key.as_deref(), Some("test-key"));
        assert_eq!(provider.language.as_deref(), Some("en"));
    }
}
