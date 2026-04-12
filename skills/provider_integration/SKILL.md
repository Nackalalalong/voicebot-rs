---
name: Provider Integration
---

# Skill: Provider Integration

Use this whenever integrating with OpenAI-compatible APIs (Speaches, vLLM, Ollama, LiteLLM) or any external streaming API.

## General rules

- Always use `reqwest` for HTTP. Never use `hyper` directly in provider crates.
- Always use `tokio-tungstenite` for WebSocket providers.
- All provider structs MUST implement the corresponding trait from `common`.
- API keys come from `SessionConfig`, never from env directly inside provider code.
- Every external call MUST have a timeout. See timeouts table below.

## Timeouts (do not adjust without updating this file)

| Provider  | Connection timeout | Response timeout | Stream idle timeout |
| --------- | ------------------ | ---------------- | ------------------- |
| Speaches  | 5s                 | 60s              | 30s                 |
| OpenAI    | 5s                 | 60s              | 30s                 |
| Anthropic | 5s                 | 60s              | 30s                 |
| Whisper   | —                  | 30s              | —                   |

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

## OpenAI-compatible ASR (Speaches)

Uses the standard OpenAI `/v1/audio/transcriptions` endpoint. Works with Speaches, OpenAI, or any compatible server.

```rust
pub struct SpeachesAsrProvider {
    base_url: String,
    model: String,
    api_key: Option<String>,
    language: Option<String>,
    client: reqwest::Client,
}

impl AsrProvider for SpeachesAsrProvider {
    async fn transcribe(
        &self,
        audio_data: Vec<u8>,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), AsrError> {
        let url = format!("{}/v1/audio/transcriptions", self.base_url);

        let mut form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .text("response_format", "json")
            .part("file", reqwest::multipart::Part::bytes(audio_data)
                .file_name("audio.wav")
                .mime_str("audio/wav")?);

        if let Some(lang) = &self.language {
            form = form.text("language", lang.clone());
        }

        let mut req = self.client.post(&url).multipart(form);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = timeout(Duration::from_secs(60), req.send())
            .await
            .map_err(|_| AsrError::Timeout)??;

        let result: TranscriptionResponse = resp.json().await?;
        tx.send(PipelineEvent::FinalTranscript {
            text: result.text,
            language: self.language.clone().unwrap_or_default(),
        }).await.ok();

        Ok(())
    }
}
```

### Streaming ASR (SSE)

Use `stream: true` to get `transcript.text.delta` events:

```rust
let form = form.text("stream", "true");
// Response is SSE: transcript.text.delta → PartialTranscript
//                  transcript.text.done  → FinalTranscript
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

## OpenAI-compatible TTS (Speaches)

Uses the standard OpenAI `/v1/audio/speech` endpoint with chunked transfer encoding for streaming.

```rust
impl TtsProvider for SpeachesTtsProvider {
    async fn synthesize(
        &self,
        mut text_rx: Receiver<String>,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), TtsError> {
        let url = format!("{}/v1/audio/speech", self.base_url);

        // Collect text to synthesize
        let mut full_text = String::new();
        while let Some(chunk) = text_rx.recv().await {
            full_text.push_str(&chunk);
        }

        let body = serde_json::json!({
            "model": self.model,
            "voice": self.voice,
            "input": full_text,
            "response_format": "pcm",  // raw 24kHz 16-bit LE
        });

        let mut req = self.client.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let response = timeout(Duration::from_secs(60), req.send())
            .await
            .map_err(|_| TtsError::Timeout)??;

        // Stream chunked PCM audio
        let mut stream = response.bytes_stream();
        let mut sequence: u32 = 0;

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|_| TtsError::StreamError)?;
            let samples = bytes_to_i16_samples(&bytes);
            let frame = AudioFrame::new(samples, 0);
            tx.send(PipelineEvent::TtsAudioChunk { frame, sequence }).await.ok();
            sequence += 1;
        }

        tx.send(PipelineEvent::TtsComplete).await.ok();
        Ok(())
    }

    async fn cancel(&self) {
        self.cancel_token.cancel();
    }
}
```

### Supported output formats

- `pcm` — raw 24kHz 16-bit signed LE (lowest latency, no decode overhead)
- `wav` — uncompressed, low latency
- `mp3` — default, general use
- `opus` — low latency internet streaming

## Adding a new provider

1. Create `crates/<component>/src/providers/<name>.rs`
2. Implement the trait from `common` (e.g., `LlmProvider`, `AsrProvider`, `TtsProvider`)
3. Add the provider enum variant to `config.toml`'s provider selection
4. Register it in the provider factory in `core::session`
5. Add an integration test in `crates/<component>/tests/<name>_integration.rs`
6. Update `CLAUDE.md` provider table

Do NOT modify the trait definitions in `common`. If the trait needs extending, discuss first.
