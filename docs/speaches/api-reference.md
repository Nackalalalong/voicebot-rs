# Speaches API Reference

> Source: `system/speaches` (commit from master branch, April 2026) Server version: 0.8.3 All endpoints require `Authorization: Bearer <key>` when `SPEACHES__API_KEY` is set, except `/health` and the WebSocket endpoint (which handles its own auth).

---

## Speech-to-Text (ASR)

### POST `/v1/audio/transcriptions`

Transcribe audio to text. Supports streaming via SSE.

**Content-Type:** `multipart/form-data`

| Field | Type | Required | Default | Notes |
| --- | --- | --- | --- | --- |
| `file` | file upload | yes | — | Audio file |
| `model` | string | yes | — | e.g. `Systran/faster-distil-whisper-large-v3` |
| `language` | string | no | `null` | BCP-47 language code |
| `prompt` | string | no | `null` | Context/prompt for the model |
| `response_format` | string | no | `"json"` | `text`, `json`, `verbose_json`, `srt`, `vtt` |
| `temperature` | float | no | `0.0` | Sampling temperature |
| `timestamp_granularities` | array | no | `["segment"]` | `segment` and/or `word` |
| `stream` | bool | no | `false` | If true, returns SSE stream |
| `hotwords` | string | no | `null` | Hotwords for improved recognition |
| `without_timestamps` | bool | no | `true` | Disable timestamp generation |

**Response (non-streaming):**

```json
// response_format=json
{ "text": "transcribed text" }

// response_format=verbose_json
{
  "task": "transcribe",
  "language": "english",
  "duration": 5.2,
  "text": "...",
  "segments": [{ "id": 0, "start": 0.0, "end": 2.5, "text": "..." }]
}
```

**Response (streaming):** `text/event-stream` with SSE events containing partial transcription segments.

---

### POST `/v1/audio/translations`

Translate audio speech to English text.

**Content-Type:** `multipart/form-data`

| Field | Type | Required | Default | Notes |
| --- | --- | --- | --- | --- |
| `file` | file upload | yes | — | Audio file |
| `model` | string | yes | — | e.g. `Systran/faster-distil-whisper-large-v3` |
| `prompt` | string | no | `null` |  |
| `response_format` | string | no | `"json"` | `text`, `json`, `verbose_json` |
| `temperature` | float | no | `0.0` |  |

**Response:**

```json
{"text": "translated text in English"}
```

---

## Text-to-Speech (TTS)

### POST `/v1/audio/speech`

Generate audio from text. Streams output.

**Content-Type:** `application/json`

```json
{
    "model": "speaches-ai/Kokoro-82M-v1.0-ONNX",
    "input": "Hello world",
    "voice": "af_heart",
    "response_format": "mp3",
    "speed": 1.0,
    "stream_format": "audio",
    "sample_rate": 24000
}
```

| Field | Type | Required | Default | Notes |
| --- | --- | --- | --- | --- |
| `model` | string | yes | — | TTS model ID |
| `input` | string | yes | — | Text to synthesize |
| `voice` | string | yes | — | Voice identifier |
| `response_format` | string | no | `"mp3"` | `pcm`, `mp3`, `wav`, `flac`, `opus`, `aac` |
| `speed` | float | no | `1.0` | Speech speed multiplier |
| `stream_format` | string | no | `"audio"` | `audio` (binary stream) or `sse` (SSE events) |
| `sample_rate` | int | no | `null` | 8000–48000 Hz |

**Response (`stream_format=audio`):** Binary audio stream with appropriate MIME type.

**Response (`stream_format=sse`):** `text/event-stream` with events:

- `audio.delta` — base64-encoded audio chunk
- `audio.done` — generation complete

---

## Chat Completions (with Audio)

### POST `/v1/chat/completions`

OpenAI-compatible chat completions. Supports audio input/output modalities.

**Content-Type:** `application/json`

```json
{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "Hello"}],
    "modalities": ["text", "audio"],
    "audio": {"format": "pcm16", "voice": "af_heart"},
    "stream": true,
    "transcription_model": "Systran/faster-distil-whisper-large-v3",
    "speech_model": "speaches-ai/Kokoro-82M-v1.0-ONNX",
    "speech_extra_body": {"sample_rate": 24000}
}
```

**Key fields:**

| Field | Type | Default | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `model` | string | — | LLM model name |
| `messages` | array | — | Chat messages (supports audio content parts) |
| `modalities` | array | `null` | `["text"]` or `["text", "audio"]` |
| `audio` | object | `null` | `{ "format": "wav | mp3 | flac | opus | pcm16", "voice": "..." }` |
| `stream` | bool | `false` | Enable SSE streaming |
| `transcription_model` | string | `"whisper-1"` | STT model for audio input |
| `speech_model` | string | `"tts-1"` | TTS model for audio output |
| `speech_extra_body` | object | `{"sample_rate": 24000}` | Extra TTS params |
| `temperature` | float | `null` |  |
| `max_tokens` | int | `null` |  |
| `tools` | array | `null` | Tool/function definitions |
| `stream_options` | object | `null` | e.g. `{"include_usage": true}` |

**Audio in messages:** User messages can include audio via `input_audio` content parts with `data` (base64) and `format` fields.

**Response (non-streaming):** Standard `ChatCompletion` JSON with optional `audio` field containing `id`, `data` (base64), `transcript`.

**Response (streaming):** `text/event-stream` with `ChatCompletionChunk` events. Audio deltas arrive as base64 in the chunk's `audio.data` field.

---

## Models

### GET `/v1/models`

List all locally loaded models.

| Query Param | Type | Default | Notes |
| --- | --- | --- | --- |
| `task` | string | `null` | Filter: `automatic-speech-recognition`, `text-to-speech`, `speaker-embedding`, `voice-activity-detection`, `speaker-diarization` |

```json
{
    "object": "list",
    "data": [
        {
            "id": "Systran/faster-distil-whisper-large-v3",
            "object": "model",
            "created": 1700000000,
            "owned_by": "Systran",
            "language": ["en"],
            "task": "automatic-speech-recognition"
        }
    ]
}
```

### GET `/v1/audio/models`

List loaded TTS models only.

### GET `/v1/audio/voices`

List available voices from loaded TTS models.

```json
{
    "object": "list",
    "voices": [{"name": "af_heart", "model": "speaches-ai/Kokoro-82M-v1.0-ONNX", "language": "en"}]
}
```

### GET `/v1/models/{model_id}`

Get info for a specific model. Returns 404 if not loaded.

### POST `/v1/models/{model_id}`

Download a model from HuggingFace Hub. Returns 201 on success, 200 if already present, 401 if gated (needs `HF_TOKEN`), 404 if not found.

### DELETE `/v1/models/{model_id}`

Unload and delete a model. Returns 200 on success, 404 if not found.

### GET `/v1/registry`

List available remote models from the registry.

| Query Param | Type   | Default | Notes          |
| ----------- | ------ | ------- | -------------- |
| `task`      | string | `null`  | Filter by task |

---

## Voice Activity Detection (VAD)

### POST `/v1/audio/speech/timestamps`

Detect speech segments in audio using Silero VAD.

**Content-Type:** `multipart/form-data`

| Field | Type | Required | Default | Notes |
| --- | --- | --- | --- | --- |
| `file` | file upload | yes | — | Audio file |
| `model` | string | no | `"silero_vad"` | Only `silero_vad` supported |
| `threshold` | float | no | `0.75` | Speech probability threshold (0–1) |
| `neg_threshold` | float | no | `null` | Silence probability threshold |
| `min_speech_duration_ms` | int | no | `0` | Min speech chunk length (ms) |
| `max_speech_duration_s` | float | no | `inf` | Max speech chunk length (s) |
| `min_silence_duration_ms` | int | no | `1000` | Min silence to split segment (ms) |
| `speech_pad_ms` | int | no | `0` | Padding around speech chunks (ms) |

```json
[
    {"start": 500, "end": 3200},
    {"start": 4100, "end": 8900}
]
```

Values are in **milliseconds**.

---

## Speaker Diarization

### POST `/v1/audio/diarization`

Identify who spoke when.

**Content-Type:** `multipart/form-data`

| Field | Type | Required | Default | Notes |
| --- | --- | --- | --- | --- |
| `file` | file upload | yes | — | Audio file |
| `model` | string | yes | — | e.g. `pyannote/speaker-diarization-3.1` |
| `known_speaker_names` | array[string] | no | `null` | Names for known speakers |
| `known_speaker_references` | array[string] | no | `null` | Reference audio as data URLs |
| `response_format` | string | no | `"json"` | `json` or `rttm` |

**Response (json):**

```json
{
    "duration": 30.5,
    "segments": [
        {"start": 0.0, "end": 2.5, "speaker": "SPEAKER_00"},
        {"start": 2.5, "end": 5.1, "speaker": "SPEAKER_01"}
    ]
}
```

**Response (rttm):** Plain text in RTTM format.

---

## Speaker Embedding

### POST `/v1/audio/speech/embedding`

Generate speaker embedding vector from audio.

**Content-Type:** `multipart/form-data`

| Field   | Type        | Required | Default | Notes                      |
| ------- | ----------- | -------- | ------- | -------------------------- |
| `file`  | file upload | yes      | —       | Audio file                 |
| `model` | string      | yes      | —       | Speaker embedding model ID |

```json
{
  "object": "list",
  "data": [
    { "embedding": [0.1, -0.2, ...], "object": "embedding", "index": 0 }
  ],
  "model": "pyannote/embedding",
  "usage": { "prompt_tokens": 160000, "total_tokens": 160000 }
}
```

---

## Realtime (WebSocket)

### WebSocket `/v1/realtime`

OpenAI Realtime API-compatible bidirectional audio/text communication.

| Query Param           | Type   | Required | Default          | Notes                             |
| --------------------- | ------ | -------- | ---------------- | --------------------------------- |
| `model`               | string | yes      | —                | e.g. `gpt-4o-realtime-preview`    |
| `intent`              | string | no       | `"conversation"` | `conversation` or `transcription` |
| `language`            | string | no       | `null`           | Language hint for STT             |
| `transcription_model` | string | no       | `null`           | Override STT model                |

**Auth:** API key via WebSocket subprotocol or query param.

**Client → Server events:**

- `session.update` — Update session configuration
- `input_audio_buffer.append` — Send base64 audio chunk
- `input_audio_buffer.commit` — Finalize audio buffer
- `input_audio_buffer.clear` — Clear audio buffer
- `conversation.item.create` — Add conversation item
- `conversation.item.truncate` — Truncate item
- `conversation.item.delete` — Delete item
- `response.create` — Request a response
- `response.cancel` — Cancel in-progress response

**Server → Client events:**

- `session.created` / `session.updated`
- `input_audio_buffer.committed` / `cleared` / `speech_started` / `speech_stopped`
- `conversation.item.created` / `truncated` / `deleted`
- `response.created` / `done`
- `response.audio.delta` / `response.audio.done`
- `response.audio_transcript.delta` / `response.audio_transcript.done`
- `response.text.delta` / `response.text.done`
- `error`

**Session lifetime:** 24 hours max.

---

## Realtime (WebRTC)

### POST `/v1/realtime` (with SDP body)

WebRTC session negotiation for real-time audio.

| Query Param | Type   | Required | Notes                          |
| ----------- | ------ | -------- | ------------------------------ |
| `model`     | string | yes      | e.g. `gpt-4o-realtime-preview` |

**Content-Type:** `text/plain` or `application/json` (SDP offer)

**Response:** SDP answer (`text/plain; charset=utf-8`)

**Protocol details:**

- Audio: 48kHz stereo in → resampled to 24kHz mono for processing
- Data channel for event messages (same event protocol as WebSocket)
- Codec: Opus only
- Message fragmentation for messages >900 bytes (`FullMessageEvent` / `PartialMessageEvent`)
- Audio buffer emits at 200ms intervals

---

## Diagnostics

### GET `/health`

Health check. No authentication required.

```json
{"message": "OK"}
```

### GET `/api/ps`

_Experimental._ List currently loaded/running models.

```json
{"models": ["Systran/faster-distil-whisper-large-v3", "speaches-ai/Kokoro-82M-v1.0-ONNX"]}
```

### POST `/api/ps/{model_id}`

_Experimental._ Load a model into memory. Returns 201 (loaded), 409 (already loaded), 404 (unknown).

### DELETE `/api/ps/{model_id}`

_Experimental._ Unload a model from memory. Returns 200 (unloaded), 404 (not loaded).
