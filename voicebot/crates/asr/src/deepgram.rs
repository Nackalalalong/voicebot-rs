use async_trait::async_trait;
use common::error::AsrError;
use common::events::PipelineEvent;
use common::traits::{AsrProvider, AudioInputStream};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc::Sender;
use tokio_tungstenite::tungstenite::Message;

pub struct DeepgramProvider {
    api_key: String,
    model: String,
    language: String,
}

impl DeepgramProvider {
    pub fn new(api_key: String, language: String) -> Self {
        Self {
            api_key,
            model: "nova-2".into(),
            language,
        }
    }

    pub fn with_model(mut self, model: String) -> Self {
        self.model = model;
        self
    }
}

#[derive(Debug, Deserialize)]
struct DeepgramResponse {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    msg_type: Option<String>,
    channel: Option<DeepgramChannel>,
    is_final: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct DeepgramChannel {
    alternatives: Vec<DeepgramAlternative>,
}

#[derive(Debug, Deserialize)]
struct DeepgramAlternative {
    transcript: String,
    confidence: f64,
}

#[async_trait]
impl AsrProvider for DeepgramProvider {
    async fn stream(
        &self,
        mut audio: Box<dyn AudioInputStream>,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), AsrError> {
        let url = format!(
            "wss://api.deepgram.com/v1/listen?\
             model={}&language={}&encoding=linear16&sample_rate=16000&channels=1&interim_results=true",
            self.model, self.language
        );

        let request = tokio_tungstenite::tungstenite::http::Request::builder()
            .uri(&url)
            .header("Authorization", format!("Token {}", self.api_key))
            .header("Host", "api.deepgram.com")
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tokio_tungstenite::tungstenite::handshake::client::generate_key(),
            )
            .body(())
            .map_err(|e| AsrError::InvalidResponse(format!("failed to build request: {e}")))?;

        let (ws_stream, _response) =
            tokio::time::timeout(std::time::Duration::from_secs(5), tokio_tungstenite::connect_async(request))
                .await
                .map_err(|_| AsrError::Timeout(5000))?
                .map_err(|_| AsrError::ConnectionFailed)?;

        tracing::info!("connected to Deepgram WebSocket");

        let (mut sink, mut stream) = ws_stream.split();

        let language = self.language.clone();

        // Sender task: read audio frames and forward as binary WS messages
        tokio::spawn(async move {
            while let Some(frame) = audio.recv().await {
                let bytes = frame.to_pcm_bytes();
                if sink.send(Message::Binary(bytes.into())).await.is_err() {
                    tracing::warn!("Deepgram WS sink closed while sending audio");
                    return;
                }
            }
            // Audio stream ended — send CloseStream
            let close_msg = r#"{"type": "CloseStream"}"#;
            if let Err(e) = sink.send(Message::Text(close_msg.into())).await {
                tracing::warn!("failed to send CloseStream to Deepgram: {e}");
            }
        });

        // Receiver loop: read WS messages and emit pipeline events
        while let Some(msg_result) = stream.next().await {
            let msg = match msg_result {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!("Deepgram WS read error: {e}");
                    break;
                }
            };

            match msg {
                Message::Text(text) => {
                    let resp: DeepgramResponse = serde_json::from_str(&text).map_err(|e| {
                        AsrError::InvalidResponse(format!("JSON parse error: {e}"))
                    })?;

                    let channel = match resp.channel {
                        Some(ch) => ch,
                        None => continue,
                    };

                    let alt = match channel.alternatives.first() {
                        Some(a) => a,
                        None => continue,
                    };

                    if alt.transcript.is_empty() {
                        continue;
                    }

                    let is_final = resp.is_final.unwrap_or(false);

                    let event = if is_final {
                        PipelineEvent::FinalTranscript {
                            text: alt.transcript.clone(),
                            language: language.clone(),
                        }
                    } else {
                        PipelineEvent::PartialTranscript {
                            text: alt.transcript.clone(),
                            confidence: alt.confidence as f32,
                        }
                    };

                    if tx.send(event).await.is_err() {
                        tracing::warn!("pipeline event channel closed");
                        break;
                    }
                }
                Message::Close(_) => {
                    tracing::info!("Deepgram WS closed by server");
                    break;
                }
                _ => {}
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_deepgram_response_final() {
        let json = r#"{
            "type": "Results",
            "channel": {
                "alternatives": [{"transcript": "hello world", "confidence": 0.95}]
            },
            "is_final": true
        }"#;
        let resp: DeepgramResponse = serde_json::from_str(json).unwrap();
        assert!(resp.is_final.unwrap_or(false));
        let alt = &resp.channel.unwrap().alternatives[0];
        assert_eq!(alt.transcript, "hello world");
        assert!((alt.confidence - 0.95).abs() < 0.01);
    }

    #[test]
    fn test_parse_deepgram_response_partial() {
        let json = r#"{
            "type": "Results",
            "channel": {
                "alternatives": [{"transcript": "hel", "confidence": 0.5}]
            },
            "is_final": false
        }"#;
        let resp: DeepgramResponse = serde_json::from_str(json).unwrap();
        assert!(!resp.is_final.unwrap_or(false));
    }

    #[test]
    fn test_provider_creation() {
        let provider = DeepgramProvider::new("test-key".into(), "en".into());
        assert_eq!(provider.model, "nova-2");
        assert_eq!(provider.language, "en");
    }

    #[test]
    fn test_provider_with_model() {
        let provider = DeepgramProvider::new("test-key".into(), "th".into())
            .with_model("nova-3".into());
        assert_eq!(provider.model, "nova-3");
    }
}
