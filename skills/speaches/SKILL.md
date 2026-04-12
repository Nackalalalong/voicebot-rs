---
name: Speaches Integration
---

# Skill: Speaches Integration

Use this whenever integrating with a local [Speaches](https://github.com/speaches-ai/speaches) server for ASR (faster-whisper), TTS (Kokoro/Piper), VAD (Silero), or the OpenAI-compatible Chat + Realtime API.

Full API reference: `docs/speaches/api-reference.md`

## General rules

- Use `reqwest` for HTTP endpoints. Use `tokio-tungstenite` for the Realtime WebSocket.
- Speaches is self-hosted — base URL comes from `SessionConfig` (e.g. `http://localhost:8000`), never hardcoded.
- API key is optional (`SPEACHES__API_KEY`). When present, send `Authorization: Bearer <key>`. The `/health` endpoint never requires auth.
- All audio entering/leaving the pipeline is `AudioFrame` (16kHz mono i16). Convert to/from Speaches formats at the boundary.
- Every external call MUST have a timeout. See timeout table below.
- Use `common::retry::with_retry` for transient failures (5xx, connection refused). Max 3 attempts.

## Timeouts

| Endpoint                      | Connection | Response | Stream idle |
| ----------------------------- | ---------- | -------- | ----------- |
| `/v1/audio/transcriptions`    | 5s         | 30s      | 10s         |
| `/v1/audio/speech`            | 5s         | —        | 10s         |
| `/v1/chat/completions`        | 5s         | 60s      | 30s         |
| `/v1/audio/speech/timestamps` | 5s         | 30s      | —           |
| `/v1/realtime` (WS)           | 5s         | —        | 10s         |
| `/health`                     | 2s         | 5s       | —           |

## AudioFrame ↔ PCM bytes conversion

Speaches expects raw audio as a file upload. Convert `AudioFrame` (i16) to little-endian PCM bytes for upload, and convert PCM bytes from TTS back to `AudioFrame`.

```rust
/// AudioFrame → PCM bytes (LE i16) for multipart upload.
fn frame_to_pcm_bytes(frame: &AudioFrame) -> Vec<u8> {
    frame.data.iter().flat_map(|s| s.to_le_bytes()).collect()
}

/// PCM bytes (LE i16) → AudioFrame.
fn pcm_bytes_to_frame(bytes: &[u8], timestamp_ms: u64) -> AudioFrame {
    AudioFrame::from_pcm_bytes(bytes, timestamp_ms)
}
```

When requesting TTS output for pipeline use, always set `response_format: "pcm"` and `sample_rate: 16000` so no codec conversion is needed.

## ASR via multipart upload

Use `/v1/audio/transcriptions` with `reqwest::multipart`. Collect buffered audio (e.g. after VAD speech-end) into PCM bytes and post as a file.

```rust
use reqwest::multipart;

pub struct SpeachesAsrProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    language: Option<String>,
}

impl SpeachesAsrProvider {
    async fn transcribe(&self, audio: &[AudioFrame]) -> Result<String, AsrError> {
        let pcm: Vec<u8> = audio.iter().flat_map(|f| frame_to_pcm_bytes(f)).collect();

        let file_part = multipart::Part::bytes(pcm)
            .file_name("audio.pcm")
            .mime_str("audio/L16;rate=16000;channels=1")
            .map_err(|_| AsrError::InvalidInput)?;

        let mut form = multipart::Form::new()
            .part("file", file_part)
            .text("model", self.model.clone())
            .text("response_format", "json");

        if let Some(lang) = &self.language {
            form = form.text("language", lang.clone());
        }

        let mut req = self.client
            .post(format!("{}/v1/audio/transcriptions", self.base_url))
            .multipart(form)
            .timeout(Duration::from_secs(30));

        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = req.send().await.map_err(|e| {
            if e.is_timeout() { AsrError::Timeout } else { AsrError::ConnectionFailed }
        })?;

        if !resp.status().is_success() {
            return Err(AsrError::ProviderError(
                format!("Speaches ASR returned {}", resp.status()),
            ));
        }

        let body: serde_json::Value = resp.json().await
            .map_err(|_| AsrError::InvalidResponse)?;
        body["text"].as_str()
            .map(|s| s.to_string())
            .ok_or(AsrError::InvalidResponse)
    }
}
```

### Streaming ASR via SSE

For real-time partial results, set `stream: true` and parse `text/event-stream`:

```rust
async fn transcribe_stream(
    &self,
    audio: &[AudioFrame],
    tx: Sender<PipelineEvent>,
) -> Result<(), AsrError> {
    let pcm: Vec<u8> = audio.iter().flat_map(|f| frame_to_pcm_bytes(f)).collect();

    let file_part = multipart::Part::bytes(pcm)
        .file_name("audio.pcm")
        .mime_str("audio/L16;rate=16000;channels=1")
        .map_err(|_| AsrError::InvalidInput)?;

    let form = multipart::Form::new()
        .part("file", file_part)
        .text("model", self.model.clone())
        .text("response_format", "verbose_json")
        .text("stream", "true");

    let mut req = self.client
        .post(format!("{}/v1/audio/transcriptions", self.base_url))
        .multipart(form)
        .timeout(Duration::from_secs(30));

    if let Some(key) = &self.api_key {
        req = req.bearer_auth(key);
    }

    let resp = req.send().await.map_err(|e| {
        if e.is_timeout() { AsrError::Timeout } else { AsrError::ConnectionFailed }
    })?;

    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|_| AsrError::StreamError)?;
        buffer.push_str(&String::from_utf8_lossy(&bytes));

        for line in buffer.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                if let Ok(segment) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(text) = segment["text"].as_str() {
                        tx.send(PipelineEvent::PartialTranscript {
                            text: text.to_string(),
                            confidence: 0.0,
                        }).await.ok();
                    }
                }
            }
        }
        buffer.clear();
    }
    Ok(())
}
```

## TTS via streaming HTTP

Use `/v1/audio/speech` with JSON body. Request `pcm` format at 16kHz to avoid codec overhead.

```rust
pub struct SpeachesTtsProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    voice: String,
    cancel_token: CancellationToken,
}

impl TtsProvider for SpeachesTtsProvider {
    async fn synthesize(
        &self,
        mut text_rx: Receiver<String>,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), TtsError> {
        let mut sequence: u32 = 0;

        while let Some(text) = text_rx.recv().await {
            if self.cancel_token.is_cancelled() { break; }

            let body = serde_json::json!({
                "model": self.model,
                "input": text,
                "voice": self.voice,
                "response_format": "pcm",
                "sample_rate": 16000,
                "stream_format": "audio",
            });

            let mut req = self.client
                .post(format!("{}/v1/audio/speech", self.base_url))
                .json(&body);

            if let Some(key) = &self.api_key {
                req = req.bearer_auth(key);
            }

            let resp = req.send().await.map_err(|e| {
                if e.is_timeout() { TtsError::Timeout } else { TtsError::ConnectionFailed }
            })?;

            if !resp.status().is_success() {
                return Err(TtsError::ProviderError(
                    format!("Speaches TTS returned {}", resp.status()),
                ));
            }

            // Stream PCM chunks as AudioFrames
            let mut stream = resp.bytes_stream();
            let mut pcm_buf = Vec::new();
            let chunk_samples = 16000 / 50; // 20ms frames = 320 samples

            while let Some(chunk) = stream.next().await {
                if self.cancel_token.is_cancelled() { break; }
                let bytes = chunk.map_err(|_| TtsError::StreamError)?;
                pcm_buf.extend_from_slice(&bytes);

                // Emit complete 20ms frames
                let frame_bytes = chunk_samples * 2; // 2 bytes per i16
                while pcm_buf.len() >= frame_bytes {
                    let frame_data: Vec<u8> = pcm_buf.drain(..frame_bytes).collect();
                    let frame = AudioFrame::from_pcm_bytes(&frame_data, 0);
                    tx.send(PipelineEvent::TtsAudioChunk { frame, sequence }).await.ok();
                    sequence += 1;
                }
            }

            // Flush remaining samples
            if !pcm_buf.is_empty() {
                let frame = AudioFrame::from_pcm_bytes(&pcm_buf, 0);
                tx.send(PipelineEvent::TtsAudioChunk { frame, sequence }).await.ok();
                sequence += 1;
            }
        }

        tx.send(PipelineEvent::TtsComplete).await.ok();
        Ok(())
    }

    async fn cancel(&self) {
        self.cancel_token.cancel();
    }
}
```

## Health check

Always verify Speaches is reachable before starting a session.

```rust
async fn check_health(client: &reqwest::Client, base_url: &str) -> Result<(), ProviderError> {
    let resp = client
        .get(format!("{}/health", base_url))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|_| ProviderError::Unavailable("Speaches health check failed"))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        Err(ProviderError::Unavailable("Speaches returned unhealthy status"))
    }
}
```

## Realtime WebSocket

For full-duplex audio, use `/v1/realtime`. This follows the OpenAI Realtime API protocol.

```rust
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

async fn connect_realtime(
    base_url: &str,
    model: &str,
    api_key: Option<&str>,
) -> Result<(SplitSink<...>, SplitStream<...>), TransportError> {
    let ws_url = format!(
        "{}/v1/realtime?model={}",
        base_url.replace("http", "ws"),
        model,
    );

    let mut request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(&ws_url);

    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    let request = request.body(()).map_err(|_| TransportError::InvalidUrl)?;

    let (ws, _) = timeout(Duration::from_secs(5), connect_async(request))
        .await
        .map_err(|_| TransportError::Timeout)?
        .map_err(|_| TransportError::ConnectionFailed)?;

    Ok(ws.split())
}
```

**Send audio:** Encode `AudioFrame` to base64 PCM and wrap in `input_audio_buffer.append`:

```rust
let pcm_bytes = frame_to_pcm_bytes(&frame);
let b64 = base64::engine::general_purpose::STANDARD.encode(&pcm_bytes);
let event = serde_json::json!({
    "type": "input_audio_buffer.append",
    "audio": b64,
});
ws_sink.send(Message::Text(event.to_string())).await?;
```

**Receive audio:** Decode `response.audio.delta` events from base64 back to `AudioFrame`:

```rust
match serde_json::from_str::<serde_json::Value>(&text) {
    Ok(event) => match event["type"].as_str() {
        Some("response.audio.delta") => {
            if let Some(b64) = event["delta"].as_str() {
                let pcm = base64::engine::general_purpose::STANDARD.decode(b64)?;
                let frame = AudioFrame::from_pcm_bytes(&pcm, 0);
                tx.send(PipelineEvent::TtsAudioChunk { frame, sequence }).await.ok();
                sequence += 1;
            }
        }
        Some("response.audio_transcript.delta") => {
            if let Some(text) = event["delta"].as_str() {
                tx.send(PipelineEvent::AgentPartialResponse {
                    text: text.to_string(),
                }).await.ok();
            }
        }
        Some("error") => {
            tracing::error!(error = %event, "Speaches realtime error");
        }
        _ => {}
    },
    Err(e) => tracing::warn!("invalid JSON from realtime WS: {e}"),
}
```

## Model management

List and preload models before session start:

```rust
/// List loaded models, optionally filtered by task.
async fn list_models(
    client: &reqwest::Client,
    base_url: &str,
    task: Option<&str>,
) -> Result<Vec<ModelInfo>, ProviderError> {
    let mut url = format!("{}/v1/models", base_url);
    if let Some(t) = task {
        url.push_str(&format!("?task={}", t));
    }
    let resp = client.get(&url).timeout(Duration::from_secs(5)).send().await?;
    let body: ModelsResponse = resp.json().await?;
    Ok(body.data)
}

/// Download / ensure a model is loaded.
async fn ensure_model(
    client: &reqwest::Client,
    base_url: &str,
    model_id: &str,
) -> Result<(), ProviderError> {
    let resp = client
        .post(format!("{}/v1/models/{}", base_url, model_id))
        .timeout(Duration::from_secs(300)) // model download can be slow
        .send()
        .await?;
    match resp.status().as_u16() {
        200 | 201 => Ok(()),
        401 => Err(ProviderError::AuthError("model is gated, set HF_TOKEN")),
        404 => Err(ProviderError::NotFound(model_id.to_string())),
        s => Err(ProviderError::ProviderError(format!("unexpected status {s}"))),
    }
}
```

## Config

Speaches-specific fields in `config.toml`:

```toml
[speaches]
base_url = "http://localhost:8000"  # required
api_key = "${SPEACHES_API_KEY}"     # optional, env var substitution

[speaches.asr]
model = "Systran/faster-distil-whisper-large-v3"
language = "en"                     # optional, auto-detect if omitted

[speaches.tts]
model = "speaches-ai/Kokoro-82M-v1.0-ONNX"
voice = "af_heart"

[speaches.realtime]
model = "gpt-4o-realtime-preview"   # only if using realtime WS mode
```

## Adding Speaches as a provider

1. Create `crates/asr/src/speaches.rs` — implement `AsrProvider` using multipart transcription
2. Create `crates/tts/src/speaches.rs` — implement `TtsProvider` using streaming `/v1/audio/speech`
3. Add `speaches` variants to provider enums in config
4. Register in provider factory in `core::session`
5. Add integration tests in `crates/asr/tests/` and `crates/tts/tests/`
6. Health-check Speaches at session startup; fail fast if unreachable

Do NOT modify the trait definitions in `common`. If the trait needs extending, discuss first.
