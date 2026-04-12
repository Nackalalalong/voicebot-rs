# Speaches Setup Guide

> Speaches is a self-hosted, OpenAI-compatible speech server providing ASR (faster-whisper), TTS (Kokoro/Piper), VAD (Silero), and a Realtime WebSocket API.

## Quick Start

```bash
cd system/speaches
docker compose -f compose.cpu.yaml up -d
```

Speaches will be available at `http://localhost:8000`. Verify with:

```bash
curl http://localhost:8000/health
# {"message":"OK"}
```

## Docker Compose (CPU)

The project uses `system/speaches/compose.cpu.yaml`:

```yaml
services:
    speaches:
        extends:
            file: compose.yaml
            service: speaches
        image: ghcr.io/speaches-ai/speaches:0.9.0-rc.3-cpu
        build:
            args:
                BASE_IMAGE: ubuntu:24.04
        user: root
        volumes:
            - ${HOME}/.cache/huggingface/hub:/home/ubuntu/.cache/huggingface/hub
```

- **CPU-only** — no NVIDIA GPU required
- **HuggingFace cache** — bind-mounted from host (`~/.cache/huggingface/hub`) so models persist across container restarts
- **Port 8000** — exposed by the base `compose.yaml`

For GPU support, use `compose.yaml` directly (requires NVIDIA runtime).

## Models

### ASR: `Systran/faster-distil-whisper-large-v3`

- **Task:** Automatic speech recognition (faster-whisper)
- **Languages:** Multilingual (optimized for English)
- **Endpoint:** `POST /v1/audio/transcriptions`
- **Download:** Auto-downloaded on first request, or pre-pull:

```bash
curl -X POST http://localhost:8000/v1/models/Systran/faster-distil-whisper-large-v3
```

### TTS: `speaches-ai/Kokoro-82M-v1.0-ONNX`

- **Task:** Text-to-speech (Kokoro ONNX)
- **Languages:** Multilingual — English (en-us, en-gb), Japanese, Chinese, Spanish, French, Hindi, Italian, Portuguese
- **Sample rate:** 24000 Hz
- **Endpoint:** `POST /v1/audio/speech`
- **Download:**

```bash
curl -X POST http://localhost:8000/v1/models/speaches-ai/Kokoro-82M-v1.0-ONNX
```

#### Available English Voices

| Voice ID     | Language | Gender | Description                |
| ------------ | -------- | ------ | -------------------------- |
| `af_heart`   | en-us    | female | **Default.** Warm, natural |
| `af_alloy`   | en-us    | female | Neutral, balanced          |
| `af_bella`   | en-us    | female | Expressive                 |
| `af_jessica` | en-us    | female | Clear, professional        |
| `af_nicole`  | en-us    | female | Friendly                   |
| `af_nova`    | en-us    | female | Bright, energetic          |
| `af_river`   | en-us    | female | Calm, flowing              |
| `af_sarah`   | en-us    | female | Conversational             |
| `af_sky`     | en-us    | female | Light, airy                |
| `am_adam`    | en-us    | male   | Clear, articulate          |
| `am_echo`    | en-us    | male   | Resonant                   |
| `am_eric`    | en-us    | male   | Professional               |
| `am_liam`    | en-us    | male   | Young, friendly            |
| `am_michael` | en-us    | male   | Authoritative              |
| `am_onyx`    | en-us    | male   | Deep, rich                 |
| `am_puck`    | en-us    | male   | Playful                    |
| `bf_alice`   | en-gb    | female | British, refined           |
| `bf_emma`    | en-gb    | female | British, warm              |
| `bf_lily`    | en-gb    | female | British, gentle            |
| `bm_daniel`  | en-gb    | male   | British, clear             |
| `bm_george`  | en-gb    | male   | British, authoritative     |

## Language Note

**We focus on English** for the Speaches provider. Kokoro TTS does not support Thai — there are no Thai TTS models available in Speaches. ASR (faster-whisper) supports Thai input but TTS output will always be English.

## Verify Models

```bash
# List all loaded models
curl http://localhost:8000/v1/models | jq '.data[] | {id, task}'

# List ASR models
curl 'http://localhost:8000/v1/models?task=automatic-speech-recognition' | jq '.data[].id'

# List TTS models
curl 'http://localhost:8000/v1/models?task=text-to-speech' | jq '.data[].id'

# List voices
curl http://localhost:8000/v1/audio/voices | jq '.voices[] | {name, language}'
```

## Test ASR

```bash
# Transcribe a WAV file
curl -X POST http://localhost:8000/v1/audio/transcriptions \
  -F "file=@tests/fixtures/audio/sine_440hz_1s.wav" \
  -F "model=Systran/faster-distil-whisper-large-v3" \
  -F "language=en"
```

## Test TTS

```bash
# Generate speech (streams PCM audio)
curl -X POST http://localhost:8000/v1/audio/speech \
  -H "Content-Type: application/json" \
  -d '{
    "model": "speaches-ai/Kokoro-82M-v1.0-ONNX",
    "input": "Hello, how can I help you today?",
    "voice": "af_heart",
    "response_format": "wav"
  }' \
  --output test_output.wav
```

## Config (config.toml)

```toml
[asr.speaches]
base_url = "http://localhost:8000"
model = "Systran/faster-distil-whisper-large-v3"
language = "en"

[tts.speaches]
base_url = "http://localhost:8000"
model = "speaches-ai/Kokoro-82M-v1.0-ONNX"
voice = "af_heart"
```

## Troubleshooting

- **Model download slow:** First request triggers download from HuggingFace Hub. The HF cache is bind-mounted so subsequent restarts are fast.
- **Out of memory:** Kokoro-82M is small (~82M params). For ASR, `faster-distil-whisper-large-v3` uses ~1.5GB RAM on CPU.
- **Port conflict:** Change mapping in `compose.cpu.yaml` if port 8000 is in use.
- **Health check fails:** Wait for model loading to complete. Check `docker logs speaches`.
