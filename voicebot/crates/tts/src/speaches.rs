use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use common::audio::AudioFrame;
use common::error::TtsError;
use common::events::PipelineEvent;
use common::traits::TtsProvider;
use futures::StreamExt;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, error, warn};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(10);

/// Speaches TTS provider using the `/v1/audio/speech` endpoint.
/// Requests raw PCM at 16kHz so no codec conversion is needed.
pub struct SpeachesTtsProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    voice: String,
    cancelled: Arc<AtomicBool>,
}

impl SpeachesTtsProvider {
    pub fn new(base_url: String, model: String, voice: String) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .unwrap_or_else(|error| {
                warn!(error = %error, "failed to build reqwest client with connect timeout; falling back to default client");
                reqwest::Client::new()
            });

        Self {
            client,
            base_url,
            api_key: None,
            model,
            voice,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_api_key(mut self, api_key: String) -> Self {
        self.api_key = Some(api_key);
        self
    }

    async fn synthesize_text(
        &self,
        text: &str,
        tx: &Sender<PipelineEvent>,
        sequence: &mut u32,
    ) -> Result<(), TtsError> {
        let body = serde_json::json!({
            "model": self.model,
            "input": text,
            "voice": self.voice,
            "response_format": "pcm",
            "sample_rate": 16000,
            "stream_format": "audio",
        });

        let mut req = self
            .client
            .post(format!("{}/v1/audio/speech", self.base_url))
            .json(&body);

        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = tokio::time::timeout(CONNECT_TIMEOUT, req.send())
            .await
            .map_err(|_| TtsError::Timeout)?
            .map_err(|e| {
                if e.is_timeout() {
                    TtsError::Timeout
                } else {
                    TtsError::ConnectionFailed
                }
            })?;

        if !resp.status().is_success() {
            return Err(TtsError::SynthesisError(format!(
                "Speaches TTS returned {}",
                resp.status()
            )));
        }

        let mut stream = resp.bytes_stream();
        let mut pcm_buf: Vec<u8> = Vec::with_capacity(1280);
        // 20ms frame at 16kHz mono i16 = 320 samples * 2 bytes = 640 bytes
        let frame_bytes: usize = 640;

        loop {
            if self.cancelled.load(Ordering::Relaxed) {
                debug!("Speaches TTS cancelled during streaming");
                return Err(TtsError::Cancelled);
            }

            let next_chunk = tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next())
                .await
                .map_err(|_| TtsError::Timeout)?;

            let Some(chunk) = next_chunk else {
                break;
            };

            let bytes = chunk.map_err(|e| {
                error!("Speaches TTS stream error: {e}");
                TtsError::SynthesisError(format!("stream read error: {e}"))
            })?;

            pcm_buf.extend_from_slice(&bytes);

            while pcm_buf.len() >= frame_bytes {
                let frame = AudioFrame::from_pcm_bytes(&pcm_buf[..frame_bytes], 0);
                pcm_buf.drain(..frame_bytes);
                tx.send(PipelineEvent::TtsAudioChunk {
                    frame,
                    sequence: *sequence,
                })
                .await
                .map_err(|_| TtsError::ChannelClosed)?;
                *sequence += 1;
            }
        }

        if !pcm_buf.is_empty() {
            let frame = AudioFrame::from_pcm_bytes(&pcm_buf, 0);
            tx.send(PipelineEvent::TtsAudioChunk {
                frame,
                sequence: *sequence,
            })
            .await
            .map_err(|_| TtsError::ChannelClosed)?;
            *sequence += 1;
        }

        Ok(())
    }
}

fn normalize_tts_input(text: &str) -> Option<String> {
    let mut normalized = String::with_capacity(text.len());
    let mut last_was_space = true;

    for ch in text.chars() {
        let mapped = match ch {
            '\r' => continue,
            '\n' | '\t' => ' ',
            '*' | '`' | '#' | '>' | '~' | '|' | '[' | ']' | '{' | '}' | '_' => ' ',
            ch if ch.is_alphanumeric() => ch,
            ch if ch.is_whitespace() => ' ',
            ch if matches!(
                ch,
                '.' | ',' | '!' | '?' | ';' | ':' | '\'' | '"' | '-' | '(' | ')' | '/' | '&'
            ) =>
            {
                ch
            }
            _ => ' ',
        };

        if mapped == ' ' {
            if !last_was_space {
                normalized.push(' ');
                last_was_space = true;
            }
            continue;
        }

        if matches!(mapped, '.' | ',' | '!' | '?' | ';' | ':' | ')' | '/')
            && normalized.ends_with(' ')
        {
            normalized.pop();
        }

        normalized.push(mapped);
        last_was_space = false;
    }

    let normalized = normalized.trim();
    if normalized.is_empty() {
        return None;
    }

    if normalized.chars().all(|ch| {
        ch.is_ascii_digit() || ch.is_whitespace() || matches!(ch, '.' | ')' | '(' | '-' | '/')
    }) {
        return None;
    }

    Some(normalized.to_owned())
}

#[async_trait]
impl TtsProvider for SpeachesTtsProvider {
    async fn synthesize(
        &self,
        mut text_rx: Receiver<String>,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), TtsError> {
        self.cancelled.store(false, Ordering::Relaxed);
        let mut sequence: u32 = 0;

        while let Some(text) = text_rx.recv().await {
            if self.cancelled.load(Ordering::Relaxed) {
                debug!("Speaches TTS cancelled");
                return Err(TtsError::Cancelled);
            }

            let Some(normalized_text) = normalize_tts_input(&text) else {
                debug!(original = %text, "skipping empty or non-speakable TTS text after normalization");
                continue;
            };

            if let Err(error) = self
                .synthesize_text(&normalized_text, &tx, &mut sequence)
                .await
            {
                match error {
                    TtsError::Cancelled | TtsError::ChannelClosed => return Err(error),
                    other => {
                        warn!(error = %other, text = %normalized_text, "skipping failed TTS sentence");
                    }
                }
            }
        }

        tx.send(PipelineEvent::TtsComplete)
            .await
            .map_err(|_| TtsError::ChannelClosed)?;
        Ok(())
    }

    async fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_builder() {
        let provider = SpeachesTtsProvider::new(
            "http://localhost:8000".into(),
            "speaches-ai/Kokoro-82M-v1.0-ONNX".into(),
            "af_heart".into(),
        )
        .with_api_key("test-key".into());

        assert_eq!(provider.base_url, "http://localhost:8000");
        assert_eq!(provider.model, "speaches-ai/Kokoro-82M-v1.0-ONNX");
        assert_eq!(provider.voice, "af_heart");
        assert_eq!(provider.api_key.as_deref(), Some("test-key"));
    }

    #[test]
    fn test_cancel_flag() {
        let provider = SpeachesTtsProvider::new(
            "http://localhost:8000".into(),
            "speaches-ai/Kokoro-82M-v1.0-ONNX".into(),
            "af_heart".into(),
        );
        assert!(!provider.cancelled.load(Ordering::Relaxed));
        provider.cancelled.store(true, Ordering::Relaxed);
        assert!(provider.cancelled.load(Ordering::Relaxed));
    }

    #[test]
    fn test_normalize_tts_input_strips_markdown_and_emoji() {
        let normalized = normalize_tts_input("**Morning:** ### 🏃 Count to 5!\nKeep going.")
            .expect("text should remain speakable");

        assert_eq!(normalized, "Morning: Count to 5! Keep going.");
    }

    #[test]
    fn test_normalize_tts_input_skips_counter_only_fragments() {
        assert_eq!(normalize_tts_input("### 🏃 2."), None);
        assert_eq!(normalize_tts_input("3)"), None);
    }
}
