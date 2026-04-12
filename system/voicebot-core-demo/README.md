# Voicebot Core Demo

Minimal browser-based demo for the voicebot pipeline.  
Captures microphone audio, sends it over WebSocket, and displays the conversation as chat.

## Quick Start

1. **Start the voicebot server:**
   ```bash
   cd voicebot && cargo run -p voicebot-server
   ```

2. **Start the Speaches backend** (if not already running):
   ```bash
   cd system/speaches && docker compose -f compose.cpu.yaml up -d
   ```

3. **Serve this directory** (any static file server works):
   ```bash
   cd system/voicebot-core-demo
   python3 -m http.server 3000
   ```

4. **Open** http://localhost:3000 in your browser.

5. **Click Connect**, then **click Mic** (or press Space) to start talking.

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
