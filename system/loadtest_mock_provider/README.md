# Loadtest with Mock Provider

Runs the full voicebot phone-call load test without real AI models.

```
[xphone virtual phones]  ─SIP/RTP─►  [Asterisk (Docker)]
                                             │ ARI
                                             ▼
                                    [voicebot-server (host)]
                                             │ HTTP
                                             ▼
                                    [mock-provider (Docker)]
                                    ASR / LLM / TTS stubs
```

## Prerequisites

- Docker with Compose plugin
- Rust toolchain (`source ~/.cargo/env`)
- `voicebot` workspace builds cleanly

## Quick start

```bash
# 1. Start Asterisk + mock-provider
docker compose -f system/loadtest_mock_provider/docker-compose.yaml up -d --build

# 2. Build release binaries (once; skip on subsequent runs)
cd voicebot
cargo build --release -p voicebot-server -p voicebot-loadtest

# 3. Start the voicebot server (leave running)
cargo run --release -p voicebot-server -- \
  ../system/loadtest_mock_provider/config.voicebot.toml

# 4. In a separate terminal — run the load test
cd voicebot
cargo run --release -p voicebot-loadtest -- \
  ../system/loadtest_mock_provider/loadtest.toml
```

Artifacts (per-call WAVs + summary JSON + Markdown report) are written to `voicebot/artifacts/loadtest/<run-id>/`.

## Tuning

| What                                 | How                                                 |
| ------------------------------------ | --------------------------------------------------- |
| Concurrency / call count             | Edit `loadtest.toml` `[campaign]`                   |
| Mock latency (simulate slow ASR/TTS) | `MOCK_LATENCY_MS=50` env on mock-provider container |
| voicebot log level                   | `RUST_LOG=info,voicebot_core=debug`                 |
| Docker bridge IP (if not 172.17.0.1) | Change `audio_host` in `config.voicebot.toml`       |

## Stopping

```bash
docker compose -f system/loadtest_mock_provider/docker-compose.yaml down
```
