use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use common::audio::AudioFrame;
use common::error::TtsError;
use common::events::PipelineEvent;
use common::traits::TtsProvider;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error};

/// ElevenLabs streaming TTS provider using WebSocket API.
pub struct ElevenLabsProvider {
    api_key: String,
    voice_id: String,
    model_id: String,
    cancelled: Arc<AtomicBool>,
}

impl ElevenLabsProvider {
    pub fn new(api_key: String, voice_id: String) -> Self {
        Self {
            api_key,
            voice_id,
            model_id: "eleven_multilingual_v2".into(),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_model(mut self, model_id: String) -> Self {
        self.model_id = model_id;
        self
    }
}

#[derive(Debug, Serialize)]
struct BosMessage {
    text: String,
    voice_settings: VoiceSettings,
    xi_api_key: String,
}

#[derive(Debug, Serialize)]
struct VoiceSettings {
    stability: f64,
    similarity_boost: f64,
}

#[derive(Debug, Serialize)]
struct TextMessage {
    text: String,
}

#[derive(Debug, Deserialize)]
struct ElevenLabsResponse {
    audio: Option<String>,
    #[serde(rename = "isFinal")]
    is_final: Option<bool>,
}

#[async_trait]
impl TtsProvider for ElevenLabsProvider {
    async fn synthesize(
        &self,
        mut text_rx: Receiver<String>,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), TtsError> {
        self.cancelled.store(false, Ordering::Relaxed);

        let url = format!(
            "wss://api.elevenlabs.io/v1/text-to-speech/{}/stream-input?model_id={}",
            self.voice_id, self.model_id
        );

        let request = tokio_tungstenite::tungstenite::http::Request::builder()
            .uri(&url)
            .header("xi-api-key", &self.api_key)
            .header("Host", "api.elevenlabs.io")
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tokio_tungstenite::tungstenite::handshake::client::generate_key(),
            )
            .body(())
            .map_err(|e| TtsError::SynthesisError(format!("failed to build request: {e}")))?;

        let connect_fut = tokio_tungstenite::connect_async(request);
        let (ws_stream, _response) =
            match tokio::time::timeout(std::time::Duration::from_secs(5), connect_fut).await {
                Ok(Ok(conn)) => conn,
                Ok(Err(_e)) => return Err(TtsError::ConnectionFailed),
                Err(_) => return Err(TtsError::Timeout),
            };

        debug!("ElevenLabs WebSocket connected");

        let (mut ws_tx, mut ws_rx) = ws_stream.split();

        // Send BOS message
        let bos = BosMessage {
            text: " ".into(),
            voice_settings: VoiceSettings {
                stability: 0.5,
                similarity_boost: 0.8,
            },
            xi_api_key: self.api_key.clone(),
        };
        let bos_json =
            serde_json::to_string(&bos).map_err(|e| TtsError::SynthesisError(e.to_string()))?;
        ws_tx
            .send(Message::Text(bos_json.into()))
            .await
            .map_err(|e| TtsError::SynthesisError(format!("failed to send BOS: {e}")))?;

        debug!("ElevenLabs BOS sent");

        let mut sequence: u32 = 0;
        let mut text_done = false;
        let cancelled = Arc::clone(&self.cancelled);

        loop {
            if cancelled.load(Ordering::Relaxed) {
                debug!("ElevenLabs synthesis cancelled");
                return Err(TtsError::Cancelled);
            }

            tokio::select! {
                text = text_rx.recv(), if !text_done => {
                    match text {
                        Some(chunk) => {
                            let msg = TextMessage { text: chunk };
                            let json = serde_json::to_string(&msg)
                                .map_err(|e| TtsError::SynthesisError(e.to_string()))?;
                            ws_tx.send(Message::Text(json.into())).await
                                .map_err(|e| TtsError::SynthesisError(format!("send text failed: {e}")))?;
                        }
                        None => {
                            // text_rx closed — send EOS
                            let eos = TextMessage { text: String::new() };
                            let json = serde_json::to_string(&eos)
                                .map_err(|e| TtsError::SynthesisError(e.to_string()))?;
                            ws_tx.send(Message::Text(json.into())).await
                                .map_err(|e| TtsError::SynthesisError(format!("send EOS failed: {e}")))?;
                            debug!("ElevenLabs EOS sent");
                            text_done = true;
                        }
                    }
                }

                ws_msg = ws_rx.next() => {
                    match ws_msg {
                        Some(Ok(Message::Text(text))) => {
                            let resp: ElevenLabsResponse = serde_json::from_str(&text)
                                .map_err(|e| TtsError::SynthesisError(format!("parse error: {e}")))?;

                            if let Some(audio_b64) = resp.audio {
                                let audio_bytes = base64::engine::general_purpose::STANDARD
                                    .decode(&audio_b64)
                                    .map_err(|e| TtsError::SynthesisError(format!("base64 decode error: {e}")))?;

                                if !audio_bytes.is_empty() {
                                    let frame = AudioFrame::from_pcm_bytes(&audio_bytes, 0);
                                    tx.send(PipelineEvent::TtsAudioChunk { frame, sequence })
                                        .await
                                        .map_err(|_| TtsError::ChannelClosed)?;
                                    sequence += 1;
                                }
                            }

                            if resp.is_final.unwrap_or(false) {
                                debug!("ElevenLabs synthesis complete (isFinal)");
                                tx.send(PipelineEvent::TtsComplete)
                                    .await
                                    .map_err(|_| TtsError::ChannelClosed)?;
                                break;
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            debug!("ElevenLabs WebSocket closed by server");
                            tx.send(PipelineEvent::TtsComplete)
                                .await
                                .map_err(|_| TtsError::ChannelClosed)?;
                            break;
                        }
                        Some(Err(e)) => {
                            error!("ElevenLabs WebSocket error: {e}");
                            tx.send(PipelineEvent::TtsComplete)
                                .await
                                .map_err(|_| TtsError::ChannelClosed)?;
                            break;
                        }
                        None => {
                            debug!("ElevenLabs WebSocket stream ended");
                            tx.send(PipelineEvent::TtsComplete)
                                .await
                                .map_err(|_| TtsError::ChannelClosed)?;
                            break;
                        }
                        // Ignore ping/pong/binary
                        _ => {}
                    }
                }
            }
        }

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
    fn test_parse_response_with_audio() {
        let json = r#"{"audio": "AABBCCDD", "isFinal": false}"#;
        let resp: ElevenLabsResponse = serde_json::from_str(json).unwrap();
        assert!(resp.audio.is_some());
        assert!(!resp.is_final.unwrap_or(false));
    }

    #[test]
    fn test_parse_response_final() {
        let json = r#"{"isFinal": true}"#;
        let resp: ElevenLabsResponse = serde_json::from_str(json).unwrap();
        assert!(resp.is_final.unwrap_or(false));
        assert!(resp.audio.is_none());
    }

    #[test]
    fn test_provider_creation() {
        let provider = ElevenLabsProvider::new("key".into(), "voice123".into());
        assert_eq!(provider.voice_id, "voice123");
        assert_eq!(provider.model_id, "eleven_multilingual_v2");
    }

    #[test]
    fn test_provider_cancel() {
        let provider = ElevenLabsProvider::new("key".into(), "voice123".into());
        assert!(!provider.cancelled.load(Ordering::Relaxed));
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { provider.cancel().await });
        assert!(provider.cancelled.load(Ordering::Relaxed));
    }
}
