# Loadtest with Mock Provider

Runs the full voicebot phone-call load test without real AI models.

```
[xphone virtual phones]  ─SIP/RTP─►  [Asterisk (Docker)]
                                             │ ARI
                                             ▼
                                    [voicebot-server (Docker)]
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
# 1. Start Asterisk + voicebot + mock-provider
docker compose -f system/loadtest_mock_provider/docker-compose.yaml up -d --build

# 2. Build the host-side loadtest runner (once; skip on subsequent runs)
cd voicebot
cargo build --release -p voicebot-loadtest

# 3. Run the load test from the host
cd voicebot
cargo run --release -p voicebot-loadtest -- \
  ../system/loadtest_mock_provider/loadtest.toml
```

Artifacts (per-call WAVs + summary JSON + Markdown report) are written to `voicebot/artifacts/loadtest/<run-id>/`.

## Tuning

| What | How |
| --- | --- |
| Concurrency / call count | Edit `loadtest.toml` `[campaign]` |
| Mock latency (simulate slow ASR/TTS) | `MOCK_LATENCY_MS=50` env on mock-provider container |
| voicebot log level | `docker logs -f loadtest-voicebot` |
| WSL host IP changed | Update `audio_host`, `backend.xphone.local_ip`, and `system/loadtest_mock_provider/asterisk/pjsip_transport.conf` |
| Telephony VAD sensitivity | Edit `[vad]` in `config.voicebot.docker.toml` |
| Time allowed for bot reply | Edit `record_after_playback_ms` in `loadtest.toml` |

## WSL2 / Docker Desktop Notes

This stack keeps Asterisk, voicebot, and the mock provider inside one Docker network so RTP between Asterisk and voicebot does not traverse the WSL2 host boundary.

- `loadtest.toml` `backend.xphone.local_ip` should match that same host IP so xphone advertises a reachable RTP address.
- `asterisk/pjsip_transport.conf` advertises that host IP back to xphone in SDP so RTP is symmetric in both directions.

If your WSL IP changes, update the two host-facing SIP settings before rerunning the campaign.

## Stopping

```bash
docker compose -f system/loadtest_mock_provider/docker-compose.yaml down
```
