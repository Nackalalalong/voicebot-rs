# Skill: WebSocket Transport

Use this whenever building or modifying the WebSocket transport adapter in `/crates/transport/websocket`, including server setup, frame parsing, session spawning, or the client-server protocol.

## Server setup

Use `axum` with WebSocket upgrade for the HTTP layer, backed by `tokio-tungstenite`.

```rust
use axum::{
    extract::ws::{WebSocket, WebSocketUpgrade, Message},
    routing::get,
    Router,
};

pub fn router() -> Router {
    Router::new().route("/session", get(ws_handler))
}

async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_connection)
}

async fn handle_connection(ws: WebSocket) {
    let session_id = Uuid::new_v4(); // Transport generates the UUID
    tracing::info!(%session_id, "new WebSocket connection");

    let (mut ws_sink, mut ws_stream) = ws.split();

    // Wait for session_start message before spawning pipeline
    let config = match wait_for_session_start(&mut ws_stream).await {
        Ok(config) => config,
        Err(e) => {
            tracing::error!(%session_id, "invalid session_start: {}", e);
            return;
        }
    };

    // Spawn pipeline session
    let mut session = match PipelineSession::start(config).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(%session_id, "failed to start session: {}", e);
            return;
        }
    };

    // Run bidirectional bridge
    run_ws_bridge(session_id, &mut session, ws_sink, ws_stream).await;

    // Cleanup
    session.terminate().await;
    tracing::info!(%session_id, "WebSocket session ended");
}
```

## Binary frame format (Client → Server)

Audio is sent as **binary WebSocket frames**: raw PCM, i16 little-endian, 16kHz mono, 320 samples per frame (20ms = 640 bytes).

```rust
fn parse_audio_frame(bytes: &[u8], timestamp_ms: u64) -> Result<AudioFrame, TransportError> {
    if bytes.len() % 2 != 0 {
        return Err(TransportError::InvalidFrameSize(bytes.len()));
    }
    let samples: Vec<i16> = bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    Ok(AudioFrame {
        data: samples.into(),
        sample_rate: 16000,
        channels: 1,
        timestamp_ms,
    })
}
```

## Text frame JSON protocol (Client → Server)

```json
{ "type": "session_start", "language": "th", "asr": "deepgram", "tts": "elevenlabs" }
{ "type": "session_end" }
```

### Parsing inbound text frames

```rust
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ClientMessage {
    #[serde(rename = "session_start")]
    SessionStart {
        language: String,
        asr: String,
        tts: String,
    },
    #[serde(rename = "session_end")]
    SessionEnd,
}

fn parse_client_message(text: &str) -> Result<ClientMessage, TransportError> {
    serde_json::from_str(text).map_err(|e| TransportError::InvalidJson(e.to_string()))
}
```

## Text frame JSON protocol (Server → Client)

```json
{ "type": "transcript_partial", "text": "สวัสดี" }
{ "type": "transcript_final",   "text": "สวัสดีครับ" }
{ "type": "agent_partial",      "text": "ผม" }
{ "type": "agent_final",        "text": "ผมช่วยอะไรได้บ้าง" }
{ "type": "error",              "code": "asr_timeout", "recoverable": true }
```

TTS audio is sent back as **binary frames** in the same format as input (PCM i16 LE, 16kHz mono).

### Translating PipelineEvent to outbound frames

```rust
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ServerMessage {
    #[serde(rename = "transcript_partial")]
    TranscriptPartial { text: String },
    #[serde(rename = "transcript_final")]
    TranscriptFinal { text: String },
    #[serde(rename = "agent_partial")]
    AgentPartial { text: String },
    #[serde(rename = "agent_final")]
    AgentFinal { text: String },
    #[serde(rename = "error")]
    Error { code: String, recoverable: bool },
}

async fn send_pipeline_event(
    sink: &mut SplitSink<WebSocket, Message>,
    event: PipelineEvent,
) -> Result<(), TransportError> {
    match event {
        PipelineEvent::PartialTranscript { text, .. } => {
            let msg = ServerMessage::TranscriptPartial { text };
            let json = serde_json::to_string(&msg)?;
            sink.send(Message::Text(json)).await?;
        }
        PipelineEvent::FinalTranscript { text, .. } => {
            let msg = ServerMessage::TranscriptFinal { text };
            let json = serde_json::to_string(&msg)?;
            sink.send(Message::Text(json)).await?;
        }
        PipelineEvent::AgentPartialResponse { text } => {
            let msg = ServerMessage::AgentPartial { text };
            let json = serde_json::to_string(&msg)?;
            sink.send(Message::Text(json)).await?;
        }
        PipelineEvent::AgentFinalResponse { text, .. } => {
            let msg = ServerMessage::AgentFinal { text };
            let json = serde_json::to_string(&msg)?;
            sink.send(Message::Text(json)).await?;
        }
        PipelineEvent::TtsAudioChunk { frame, .. } => {
            let bytes: Vec<u8> = frame.data.iter()
                .flat_map(|s| s.to_le_bytes())
                .collect();
            sink.send(Message::Binary(bytes)).await?;
        }
        PipelineEvent::ComponentError { component, error, recoverable } => {
            let msg = ServerMessage::Error {
                code: format!("{}_{}", component, error),
                recoverable,
            };
            let json = serde_json::to_string(&msg)?;
            sink.send(Message::Text(json)).await?;
        }
        _ => {} // Internal events not forwarded to client
    }
    Ok(())
}
```

## Bidirectional bridge loop

```rust
async fn run_ws_bridge(
    session_id: Uuid,
    session: &mut PipelineSession,
    mut sink: SplitSink<WebSocket, Message>,
    mut stream: SplitStream<WebSocket>,
) {
    let mut frame_counter: u64 = 0;

    loop {
        tokio::select! {
            // Inbound: client → pipeline
            Some(msg) = stream.next() => {
                match msg {
                    Ok(Message::Binary(bytes)) => {
                        let timestamp_ms = frame_counter * 20; // 20ms per frame
                        frame_counter += 1;
                        match parse_audio_frame(&bytes, timestamp_ms) {
                            Ok(frame) => {
                                // Use try_send — audio frames are droppable
                                if let Err(e) = session.audio_tx.try_send(frame) {
                                    tracing::warn!(%session_id, "audio channel full, dropping frame");
                                }
                            }
                            Err(e) => {
                                tracing::warn!(%session_id, "invalid audio frame: {}", e);
                            }
                        }
                    }
                    Ok(Message::Text(text)) => {
                        match parse_client_message(&text) {
                            Ok(ClientMessage::SessionEnd) => {
                                tracing::info!(%session_id, "client requested session end");
                                break;
                            }
                            Ok(_) => {} // session_start already handled
                            Err(e) => {
                                tracing::warn!(%session_id, "invalid client message: {}", e);
                            }
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Err(e) => {
                        tracing::error!(%session_id, "WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }

            // Outbound: pipeline → client
            Some(event) = session.event_rx.recv() => {
                if let Err(e) = send_pipeline_event(&mut sink, event).await {
                    tracing::error!(%session_id, "failed to send to client: {}", e);
                    break;
                }
            }

            else => break,
        }
    }
}
```

## No-leakage rule

Transport adapters MUST:

1. **Convert ALL audio** to `AudioFrame` before passing to core
2. **Translate `PipelineEvent`** into transport-native signals (JSON text / binary audio), never the reverse
3. **Never import** from `/crates/core` internals — only from `/crates/common`
4. Core pipeline MUST NOT see `axum::extract::ws::Message`, `tokio_tungstenite::Message`, or any WS-specific type
5. Session UUID is generated by the transport adapter, **not** by core

```
┌─────────────┐     AudioFrame      ┌──────────┐
│  WebSocket  │ ──────────────────► │   Core   │
│  Adapter    │                     │ Pipeline │
│             │ ◄────────────────── │          │
│  (axum/ws)  │   PipelineEvent     │          │
└─────────────┘                     └──────────┘
     ▲  │
     │  │  WS frames (binary PCM / JSON text)
     │  ▼
  [ Client ]
```

## Error type

```rust
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("invalid frame size: {0} bytes (expected even)")]
    InvalidFrameSize(usize),

    #[error("invalid JSON message: {0}")]
    InvalidJson(String),

    #[error("WebSocket error: {0}")]
    WebSocket(#[from] axum::Error),

    #[error("session start timeout")]
    SessionStartTimeout,
}
```

## What NOT to do

```rust
// Never pass WS Message types into core
fn handle(msg: axum::extract::ws::Message, pipeline: &Core) // ← forbidden

// Never generate session UUIDs inside core
let id = Uuid::new_v4(); // ← only in transport adapter

// Never import transport crates from core
use transport_websocket::WsSession; // ← forbidden in /crates/core

// Never send raw bytes without converting to AudioFrame
session.audio_tx.send(raw_bytes).await; // ← forbidden: convert first
```
