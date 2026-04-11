use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use common::audio::AudioFrame;
use common::events::{PipelineEvent, SessionConfig, VadConfig};
use common::types::{AsrProviderType, Language, LlmProviderType, TtsProviderType};
use futures::{SinkExt, StreamExt};
use uuid::Uuid;
use voicebot_core::session::PipelineSession;

use crate::error::TransportError;
use crate::protocol::{parse_client_message, ClientMessage, ServerMessage};

/// Build the Axum router with the `/session` WebSocket endpoint.
pub fn router() -> Router {
    Router::new().route("/session", get(ws_handler))
}

/// Axum handler that upgrades an HTTP request to a WebSocket connection.
async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_connection)
}

/// Main connection handler. Manages the full lifecycle of a single session:
/// 1. Generate a unique session ID
/// 2. Wait for a `session_start` JSON message from the client (10s timeout)
/// 3. Spawn the core pipeline (VAD → ASR → Agent → TTS)
/// 4. Bridge WebSocket frames ↔ pipeline events until disconnect
/// 5. Terminate the pipeline on exit
async fn handle_connection(ws: WebSocket) {
    // Transport layer owns the session UUID — core never generates it
    let session_id = Uuid::new_v4();
    tracing::info!(%session_id, "new WebSocket connection");

    let (mut ws_sink, mut ws_stream) = ws.split();

    // Wait for session_start with 10s timeout
    let config = match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        wait_for_session_start(&mut ws_stream, session_id),
    )
    .await
    {
        Ok(Ok(config)) => config,
        Ok(Err(e)) => {
            tracing::error!(%session_id, "invalid session_start: {}", e);
            return;
        }
        Err(_) => {
            tracing::error!(%session_id, "session_start timeout");
            return;
        }
    };

    // Bounded channels between transport and pipeline:
    // - audio_tx/rx: client PCM audio → VAD (capacity 50, drop on overflow)
    // - egress_tx/rx: pipeline events → client WS frames (capacity 200)
    let (audio_tx, audio_rx) = tokio::sync::mpsc::channel::<AudioFrame>(50);
    let (egress_tx, mut egress_rx) = tokio::sync::mpsc::channel::<PipelineEvent>(200);

    // Start pipeline session
    let mut session = match PipelineSession::start_with_stubs(config, audio_rx, egress_tx).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(%session_id, "failed to start session: {}", e);
            return;
        }
    };

    // Run bidirectional bridge
    run_ws_bridge(
        session_id,
        &audio_tx,
        &mut egress_rx,
        &mut ws_sink,
        &mut ws_stream,
    )
    .await;

    // Cleanup
    session.terminate().await;
    tracing::info!(%session_id, "WebSocket session ended");
}

/// Read WS text frames until we receive a valid `session_start` message.
/// Returns the parsed `SessionConfig`. Caller is responsible for the timeout.
async fn wait_for_session_start(
    ws_stream: &mut futures::stream::SplitStream<WebSocket>,
    session_id: Uuid,
) -> Result<SessionConfig, TransportError> {
    while let Some(msg) = ws_stream.next().await {
        match msg {
            Ok(Message::Text(text)) => match parse_client_message(&text) {
                Ok(ClientMessage::SessionStart { language, asr, tts }) => {
                    tracing::info!(%session_id, %language, %asr, %tts, "session_start received");
                    return Ok(SessionConfig {
                        session_id,
                        language: Language::from_str_loose(&language),
                        asr_provider: AsrProviderType::from_str_loose(&asr),
                        tts_provider: TtsProviderType::from_str_loose(&tts),
                        llm_provider: LlmProviderType::OpenAi,
                        vad_config: VadConfig::default(),
                    });
                }
                Ok(_) => {
                    tracing::warn!(%session_id, "expected session_start, got other message");
                }
                Err(e) => return Err(e),
            },
            Ok(Message::Close(_)) => {
                return Err(TransportError::Session(
                    "connection closed before session_start".into(),
                ));
            }
            Err(e) => return Err(TransportError::WebSocket(e)),
            _ => {} // ignore binary/ping/pong while waiting
        }
    }
    Err(TransportError::Session(
        "stream ended before session_start".into(),
    ))
}

/// Bidirectional bridge between the WebSocket and pipeline channels.
///
/// Inbound (client → pipeline): binary frames are parsed as PCM audio and
/// forwarded via `audio_tx`; text frames are parsed as JSON control messages.
///
/// Outbound (pipeline → client): `PipelineEvent`s are translated to JSON
/// text frames or binary audio frames and sent via `ws_sink`.
///
/// The loop exits on client disconnect, `session_end`, or send failure.
async fn run_ws_bridge(
    session_id: Uuid,
    audio_tx: &tokio::sync::mpsc::Sender<AudioFrame>,
    egress_rx: &mut tokio::sync::mpsc::Receiver<PipelineEvent>,
    ws_sink: &mut futures::stream::SplitSink<WebSocket, Message>,
    ws_stream: &mut futures::stream::SplitStream<WebSocket>,
) {
    let mut frame_counter: u64 = 0; // monotonic counter → derives timestamp_ms (20ms per frame)
    loop {
        tokio::select! {
            msg = ws_stream.next() => {
                match msg {
                    Some(Ok(Message::Binary(bytes))) => {
                        let timestamp_ms = frame_counter * 20;
                        frame_counter += 1;
                        match parse_audio_frame(&bytes, timestamp_ms) {
                            Ok(frame) => {
                                if audio_tx.try_send(frame).is_err() {
                                    tracing::warn!(%session_id, "audio channel full, dropping frame");
                                }
                            }
                            Err(e) => {
                                tracing::warn!(%session_id, "invalid audio frame: {}", e);
                            }
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        match parse_client_message(&text) {
                            Ok(ClientMessage::SessionEnd) => {
                                tracing::info!(%session_id, "client requested session end");
                                break;
                            }
                            Ok(_) => {}
                            Err(e) => {
                                tracing::warn!(%session_id, "invalid client message: {}", e);
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(e)) => {
                        tracing::error!(%session_id, "WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
            event = egress_rx.recv() => {
                match event {
                    Some(ev) => {
                        if let Err(e) = send_pipeline_event(ws_sink, ev).await {
                            tracing::error!(%session_id, "failed to send to client: {}", e);
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }
}

/// Convert raw binary WS frame bytes into an `AudioFrame`.
/// Expects PCM i16 little-endian, 16 kHz mono (so byte count must be even).
fn parse_audio_frame(bytes: &[u8], timestamp_ms: u64) -> Result<AudioFrame, TransportError> {
    if bytes.len() % 2 != 0 {
        return Err(TransportError::InvalidFrameSize(bytes.len()));
    }
    Ok(AudioFrame::from_pcm_bytes(bytes, timestamp_ms))
}

/// Translate a `PipelineEvent` into a WebSocket frame and send it.
/// - Transcripts & agent responses → JSON text frames
/// - TTS audio chunks → binary PCM frames
/// - Component errors → JSON error frames
/// - Internal events (SpeechStarted, Cancel, etc.) are silently dropped.
async fn send_pipeline_event(
    sink: &mut futures::stream::SplitSink<WebSocket, Message>,
    event: PipelineEvent,
) -> Result<(), TransportError> {
    match event {
        PipelineEvent::PartialTranscript { text, .. } => {
            let msg = ServerMessage::TranscriptPartial { text };
            let json = serde_json::to_string(&msg)
                .map_err(|e| TransportError::InvalidJson(e.to_string()))?;
            sink.send(Message::Text(json.into())).await?;
        }
        PipelineEvent::FinalTranscript { text, .. } => {
            let msg = ServerMessage::TranscriptFinal { text };
            let json = serde_json::to_string(&msg)
                .map_err(|e| TransportError::InvalidJson(e.to_string()))?;
            sink.send(Message::Text(json.into())).await?;
        }
        PipelineEvent::AgentPartialResponse { text } => {
            let msg = ServerMessage::AgentPartial { text };
            let json = serde_json::to_string(&msg)
                .map_err(|e| TransportError::InvalidJson(e.to_string()))?;
            sink.send(Message::Text(json.into())).await?;
        }
        PipelineEvent::AgentFinalResponse { text, .. } => {
            let msg = ServerMessage::AgentFinal { text };
            let json = serde_json::to_string(&msg)
                .map_err(|e| TransportError::InvalidJson(e.to_string()))?;
            sink.send(Message::Text(json.into())).await?;
        }
        PipelineEvent::TtsAudioChunk { frame, .. } => {
            let bytes = frame.to_pcm_bytes();
            sink.send(Message::Binary(bytes.into())).await?;
        }
        PipelineEvent::ComponentError {
            component,
            error,
            recoverable,
        } => {
            let msg = ServerMessage::Error {
                code: format!("{}_{}", component, error),
                recoverable,
            };
            let json = serde_json::to_string(&msg)
                .map_err(|e| TransportError::InvalidJson(e.to_string()))?;
            sink.send(Message::Text(json.into())).await?;
        }
        _ => {} // Internal events not forwarded
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_audio_frame_valid() {
        let samples: Vec<i16> = vec![100, -100, 200];
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let frame = parse_audio_frame(&bytes, 0).unwrap();
        assert_eq!(frame.data.len(), 3);
    }

    #[test]
    fn test_parse_audio_frame_odd_bytes() {
        let bytes = vec![0u8, 1, 2]; // 3 bytes = odd
        assert!(parse_audio_frame(&bytes, 0).is_err());
    }
}
