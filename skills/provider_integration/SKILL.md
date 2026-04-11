---
name: Provider Integration
---

# Skill: Provider Integration

Use this whenever integrating with Deepgram, OpenAI, Anthropic, ElevenLabs, or any external streaming API.

## General rules

- Always use `reqwest` for HTTP. Never use `hyper` directly in provider crates.
- Always use `tokio-tungstenite` for WebSocket providers.
- All provider structs MUST implement the corresponding trait from `common`.
- API keys come from `SessionConfig`, never from env directly inside provider code.
- Every external call MUST have a timeout. See timeouts table below.

## Timeouts (do not adjust without updating this file)

| Provider   | Connection timeout | Response timeout | Stream idle timeout |
| ---------- | ------------------ | ---------------- | ------------------- |
| Deepgram   | 5s                 | —                | 10s                 |
| OpenAI     | 5s                 | 60s              | 30s                 |
| Anthropic  | 5s                 | 60s              | 30s                 |
| ElevenLabs | 5s                 | —                | 10s                 |
| Whisper    | —                  | 30s              | —                   |

## Retry logic (shared helper)

```rust
// In common::retry
pub async fn with_retry<F, Fut, T, E>(
    max_attempts: u32,
    base_delay_ms: u64,
    mut f: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Debug,
{
    let mut attempt = 0;
    loop {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if attempt + 1 >= max_attempts => return Err(e),
            Err(e) => {
                tracing::warn!(attempt, error = ?e, "retrying after error");
                let delay = base_delay_ms * 2u64.pow(attempt);
                tokio::time::sleep(Duration::from_millis(delay)).await;
                attempt += 1;
            }
        }
    }
}
```

## Deepgram streaming ASR

```rust
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

pub struct DeepgramProvider {
    api_key: String,
    model: String,
    language: String,
}

impl AsrProvider for DeepgramProvider {
    async fn stream(
        &self,
        mut audio_rx: impl AudioInputStream,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), AsrError> {
        let url = format!(
            "wss://api.deepgram.com/v1/listen?model={}&language={}&encoding=linear16&sample_rate=16000&channels=1&interim_results=true",
            self.model, self.language
        );

        let request = tokio_tungstenite::tungstenite::http::Request::builder()
            .uri(&url)
            .header("Authorization", format!("Token {}", self.api_key))
            .body(())
            .map_err(|_| AsrError::ConnectionFailed)?;

        let (mut ws, _) = timeout(Duration::from_secs(5), connect_async(request))
            .await
            .map_err(|_| AsrError::Timeout)?
            .map_err(|_| AsrError::ConnectionFailed)?;

        // Spawn send task
        let (send_tx, mut send_rx) = mpsc::channel::<Vec<u8>>(50);
        tokio::spawn(async move {
            while let Some(bytes) = send_rx.recv().await {
                let _ = ws_sender.send(Message::Binary(bytes)).await;
            }
        });

        // Receive loop
        loop {
            tokio::select! {
                Some(frame) = audio_rx.recv() => {
                    let bytes = frame_to_bytes(&frame);
                    let _ = send_tx.try_send(bytes);
                }
                Some(msg) = ws_receiver.next() => {
                    match msg? {
                        Message::Text(json) => {
                            let result: DeepgramResponse = serde_json::from_str(&json)?;
                            if result.is_final {
                                tx.send(PipelineEvent::FinalTranscript {
                                    text: result.transcript.clone(),
                                    language: self.language.clone(),
                                }).await.ok();
                            } else {
                                tx.send(PipelineEvent::PartialTranscript {
                                    text: result.transcript,
                                    confidence: result.confidence,
                                }).await.ok();
                            }
                        }
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
                else => break,
            }
        }
        Ok(())
    }
}
```

## OpenAI streaming chat completions

```rust
pub struct OpenAiProvider {
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl LlmProvider for OpenAiProvider {
    async fn stream_completion(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: Sender<PipelineEvent>,
    ) -> Result<(), LlmError> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "tools": tools,
            "stream": true,
            "max_tokens": 512,
        });

        let mut stream = timeout(
            Duration::from_secs(60),
            self.client
                .post("https://api.openai.com/v1/chat/completions")
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
        ).await
        .map_err(|_| LlmError::Timeout)??
        .bytes_stream();

        let mut full_text = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|_| LlmError::StreamError)?;
            let text = std::str::from_utf8(&bytes)?;

            for line in text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" { break; }
                    if let Ok(delta) = serde_json::from_str::<StreamDelta>(data) {
                        if let Some(content) = delta.content() {
                            full_text.push_str(&content);
                            tx.send(PipelineEvent::AgentPartialResponse {
                                text: content,
                            }).await.ok();
                        }
                        if let Some(tc) = delta.tool_call() {
                            tool_calls.push(tc);
                        }
                    }
                }
            }
        }

        tx.send(PipelineEvent::AgentFinalResponse {
            text: full_text,
            tool_calls,
        }).await.ok();

        Ok(())
    }
}
```

## ElevenLabs streaming TTS

```rust
impl TtsProvider for ElevenLabsProvider {
    async fn synthesize(
        &self,
        mut text_rx: Receiver<String>,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), TtsError> {
        let url = format!(
            "wss://api.elevenlabs.io/v1/text-to-speech/{}/stream-input?model_id=eleven_multilingual_v2",
            self.voice_id
        );

        let (mut ws_sink, mut ws_stream) = connect_ws_with_auth(&url, &self.api_key).await?;

        // Send BOS
        ws_sink.send(Message::Text(serde_json::to_string(&json!({
            "text": " ",
            "voice_settings": { "stability": 0.5, "similarity_boost": 0.8 }
        }))?)).await?;

        let mut sequence: u32 = 0;

        loop {
            tokio::select! {
                Some(text_chunk) = text_rx.recv() => {
                    ws_sink.send(Message::Text(serde_json::to_string(&json!({
                        "text": text_chunk,
                    }))?)).await?;
                }
                Some(msg) = ws_stream.next() => {
                    match msg? {
                        Message::Text(json) => {
                            let resp: ElevenLabsChunk = serde_json::from_str(&json)?;
                            if let Some(audio_b64) = resp.audio {
                                let pcm = decode_mp3_to_pcm(&base64::decode(audio_b64)?)?;
                                let frame = AudioFrame {
                                    data: pcm.into(),
                                    sample_rate: 16000,
                                    channels: 1,
                                    timestamp_ms: 0,
                                };
                                tx.send(PipelineEvent::TtsAudioChunk { frame, sequence }).await.ok();
                                sequence += 1;
                            }
                            if resp.is_final.unwrap_or(false) {
                                tx.send(PipelineEvent::TtsComplete).await.ok();
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                else => break,
            }
        }
        Ok(())
    }

    async fn cancel(&self) {
        self.cancel_token.cancel();
    }
}
```

## Adding a new provider

1. Create `crates/<component>/src/providers/<name>.rs`
2. Implement the trait from `common` (e.g., `LlmProvider`, `AsrProvider`, `TtsProvider`)
3. Add the provider enum variant to `config.toml`'s provider selection
4. Register it in the provider factory in `core::session`
5. Add an integration test in `crates/<component>/tests/<name>_integration.rs`
6. Update `CLAUDE.md` provider table

Do NOT modify the trait definitions in `common`. If the trait needs extending, discuss first.
