use async_trait::async_trait;
use common::error::AsrError;
use common::events::PipelineEvent;
use common::traits::{AsrProvider, AudioInputStream};
use reqwest::multipart;
use tokio::sync::mpsc::Sender;

/// Speaches ASR provider using the `/v1/audio/transcriptions` endpoint.
pub struct SpeachesAsrProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    language: Option<String>,
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
}

#[derive(serde::Deserialize)]
struct TranscriptionResponse {
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

        if pcm_bytes.is_empty() {
            return Ok(());
        }

        let file_part = multipart::Part::bytes(pcm_bytes)
            .file_name("audio.pcm")
            .mime_str("audio/L16;rate=16000;channels=1")
            .map_err(|e| AsrError::InvalidResponse(format!("mime error: {e}")))?;

        let mut form = multipart::Form::new()
            .part("file", file_part)
            .text("model", self.model.clone())
            .text("response_format", "json");

        if let Some(lang) = &self.language {
            form = form.text("language", lang.clone());
        }

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

        let body: TranscriptionResponse = resp
            .json()
            .await
            .map_err(|e| AsrError::InvalidResponse(format!("JSON parse error: {e}")))?;

        if !body.text.is_empty() {
            let language = self.language.clone().unwrap_or_else(|| "auto".into());
            tx.send(PipelineEvent::FinalTranscript {
                text: body.text,
                language,
            })
            .await
            .map_err(|_| AsrError::ChannelClosed)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transcription_response_parse() {
        let json = r#"{"text": "hello world"}"#;
        let resp: TranscriptionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.text, "hello world");
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
