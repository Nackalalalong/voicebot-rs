use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use common::audio::AudioFrame;
use common::error::TtsError;
use common::events::PipelineEvent;
use common::traits::TtsProvider;
use futures::StreamExt;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, error};

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
        Self {
            client: reqwest::Client::new(),
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

            if text.is_empty() {
                continue;
            }

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
                .json(&body)
                .timeout(std::time::Duration::from_secs(60));

            if let Some(key) = &self.api_key {
                req = req.bearer_auth(key);
            }

            let resp = req.send().await.map_err(|e| {
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

            // Stream PCM chunks back as AudioFrames
            let mut stream = resp.bytes_stream();
            let mut pcm_buf: Vec<u8> = Vec::new();
            // 20ms frame at 16kHz mono i16 = 320 samples * 2 bytes = 640 bytes
            let frame_bytes: usize = 640;

            while let Some(chunk) = stream.next().await {
                if self.cancelled.load(Ordering::Relaxed) {
                    debug!("Speaches TTS cancelled during streaming");
                    return Err(TtsError::Cancelled);
                }

                let bytes = chunk.map_err(|e| {
                    error!("Speaches TTS stream error: {e}");
                    TtsError::SynthesisError(format!("stream read error: {e}"))
                })?;

                pcm_buf.extend_from_slice(&bytes);

                // Emit complete 20ms frames
                while pcm_buf.len() >= frame_bytes {
                    let frame_data: Vec<u8> = pcm_buf.drain(..frame_bytes).collect();
                    let frame = AudioFrame::from_pcm_bytes(&frame_data, 0);
                    tx.send(PipelineEvent::TtsAudioChunk { frame, sequence })
                        .await
                        .map_err(|_| TtsError::ChannelClosed)?;
                    sequence += 1;
                }
            }

            // Flush remaining samples (last partial frame)
            if !pcm_buf.is_empty() {
                let frame = AudioFrame::from_pcm_bytes(&pcm_buf, 0);
                tx.send(PipelineEvent::TtsAudioChunk { frame, sequence })
                    .await
                    .map_err(|_| TtsError::ChannelClosed)?;
                sequence += 1;
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
            "kokoro".into(),
            "af_heart".into(),
        )
        .with_api_key("test-key".into());

        assert_eq!(provider.base_url, "http://localhost:8000");
        assert_eq!(provider.model, "kokoro");
        assert_eq!(provider.voice, "af_heart");
        assert_eq!(provider.api_key.as_deref(), Some("test-key"));
    }

    #[test]
    fn test_cancel_flag() {
        let provider = SpeachesTtsProvider::new(
            "http://localhost:8000".into(),
            "kokoro".into(),
            "af_heart".into(),
        );
        assert!(!provider.cancelled.load(Ordering::Relaxed));
        provider.cancelled.store(true, Ordering::Relaxed);
        assert!(provider.cancelled.load(Ordering::Relaxed));
    }
}
