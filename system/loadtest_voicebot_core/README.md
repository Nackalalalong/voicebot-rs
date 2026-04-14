# Voicebot Core WebSocket Load Test

Measures latency and audio quality of **voicebot-core** directly over WebSocket — no Asterisk, no SIP, no RTP.

```
[loadtest runner (Docker)] ──ws://voicebot:8080/session──►
                                 [voicebot-server (Docker):8080]
                                           │ HTTP
                                           ▼
                                 [mock-provider (Docker):8000]
                                   ASR · LLM · TTS stubs
```

## Prerequisites

- Docker with Compose plugin
- Rust toolchain is no longer required for the default path.

## Quick start

```bash
# From the project root — builds Docker images and runs the full campaign:
system/loadtest_voicebot_core/start.sh
```

For performance runs, leave `RUST_LOG` unset so the stack uses the compose default `info` level. If you need pipeline debugging, override it explicitly, for example:

```bash
RUST_LOG=info,voicebot_core=debug,asr=debug,tts=debug,agent=debug \
system/loadtest_voicebot_core/start.sh
```

Or step by step:

```bash
# 1. Start the backing services
docker compose -f system/loadtest_voicebot_core/docker-compose.yaml up -d --build voicebot mock-provider

# 2. Run the loadtest service on the internal compose network
docker compose -f system/loadtest_voicebot_core/docker-compose.yaml --profile loadtest run --rm loadtest
```

Artifacts are written to `voicebot/artifacts/loadtest/<run-id>/`:

| File                   | Contents                                 |
| ---------------------- | ---------------------------------------- |
| `calls/NNNN/rx.wav`    | Full conversation audio for that session |
| `summary.json`         | Latency + voiced-audio statistics        |
| `report.md`            | Human-readable campaign report           |
| `tx.normalized.wav`    | Normalised TX audio used for all calls   |
| `config.resolved.toml` | Resolved config snapshot                 |

## Configuration

| File                   | Purpose                                                          |
| ---------------------- | ---------------------------------------------------------------- |
| `loadtest.toml`        | Campaign parameters (concurrency, call count, etc.)              |
| `config.voicebot.toml` | Voicebot server config (injected at `/etc/voicebot/config.toml`) |

Key `loadtest.toml` knobs:

| Setting | Default | Description |
| --- | --- | --- |
| `campaign.concurrency` | `3` | Simultaneous WebSocket sessions |
| `campaign.total_calls` | `9` | Total sessions to run (0 + `soak_duration_secs` for soak mode) |
| `campaign.record_after_playback_ms` | `8000` | How long to keep recording after TX ends |
| `campaign.ramp_up_ms` | `0` | Spread initial sessions over this window |
| `campaign.call_rate_per_second` | `0.0` | Max sessions/s (0 = unlimited) |

## Reading the results

```
calls: 9 total / 9 successful / 0 failed
duration: 12.4s
first_response p50/p90/p99: 210/340/410 ms
avg_longest_gap_ms: 18
total_stutter_count: 0
report: artifacts/loadtest/20260414_120000/report.md
```

- **first_response_ms** — time from end of TX audio to first voiced TTS frame.
- **longest_gap_ms** — longest silence gap in the received TTS stream (indicates stutter/backpressure).
- **stutter_count** — gaps < 200 ms between voiced regions (choppy audio indicator).

## Soak testing

```toml
# loadtest.toml
[campaign]
concurrency      = 5
total_calls      = 0      # unlimited
soak_duration_secs = 300  # run for 5 minutes
```

The compose file still publishes the server for debugging on `localhost:18080` and metrics on `localhost:18081`, but the loadtest service itself uses the internal compose address `voicebot:8080`.

## Stopping

```bash
docker compose -f system/loadtest_voicebot_core/docker-compose.yaml down
```
