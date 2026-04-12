# Voicebot Core Demo

Minimal browser-based demo for the voicebot pipeline.  
Captures microphone audio, sends it over WebSocket, and displays the conversation as chat.

## Quick Start

**One-command start** — builds the server, starts Speaches in Docker, and serves the web app:

```bash
cd system/voicebot-core-demo && ./start.sh
```

Then open **http://localhost:3000**, click **Connect**, and press **Mic** (or Space) to talk.

### Options

| Flag                | Default | Description                              |
| ------------------- | ------- | ---------------------------------------- |
| `--no-build`        | —       | Skip `cargo build` (use existing binary) |
| `--web-port PORT`   | `3000`  | Port for the static file server          |
| `WEB_PORT=...`      | `3000`  | Same as `--web-port` via env var         |
| `VOICEBOT_PORT=...` | `8080`  | Override voicebot server port            |

### Manual Start (alternative)

<details>
<summary>Start each component individually</summary>

1. **Speaches** (Docker):

    ```bash
    cd system/speaches && docker compose -f compose.cpu.yaml up -d
    ```

2. **Voicebot server:**

    ```bash
    cd voicebot && cargo run --release -p voicebot-server
    ```

3. **Web server** (any static file server):

    ```bash
    cd system/voicebot-core-demo && python3 -m http.server 3000
    ```

4. Open http://localhost:3000.

</details>

## How It Works

```
Browser Mic → PCM i16 16kHz → WebSocket binary frames → Voicebot Server
                                                              ↓
Browser Speaker ← PCM i16 16kHz ← WebSocket binary frames ← TTS
                                                              ↓
Chat UI ← JSON text frames ← Transcripts + Agent responses
```

### WebSocket Protocol

**Client → Server:**

- `{"type":"session_start","language":"en","asr":"speaches","tts":"speaches"}` — start session
- Binary frames: raw PCM i16 LE, 16kHz mono, 320 samples (640 bytes) per frame
- `{"type":"session_end"}` — end session

**Server → Client:**

- `{"type":"transcript_partial","text":"..."}` — partial ASR result
- `{"type":"transcript_final","text":"..."}` — final ASR result
- `{"type":"agent_partial","text":"..."}` — streaming agent response
- `{"type":"agent_final","text":"..."}` — complete agent response
- Binary frames: TTS audio, raw PCM i16 LE, 16kHz mono
- `{"type":"error","code":"...","recoverable":true}` — error

## Requirements

- Modern browser with WebSocket and getUserMedia support (Chrome, Firefox, Edge)
- Voicebot server running on `ws://localhost:8080/session` (configurable in UI)
