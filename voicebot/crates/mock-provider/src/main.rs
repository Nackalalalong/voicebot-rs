//! Mock ASR / LLM / TTS server.
//!
//! Implements the minimum subset of OpenAI-compatible HTTP endpoints consumed
//! by the voicebot pipeline so that load tests run without real AI models.
//!
//! Endpoints
//! ---------
//! GET  /health                       → {"status":"ok"}
//! GET  /v1/models                    → OpenAI-style model list
//! POST /v1/audio/transcriptions      → non-streaming verbose_json transcript
//! POST /v1/chat/completions          → SSE streaming chat completion
//! POST /v1/audio/speech              → raw PCM-16 LE at 16 kHz (20 ms of silence per call,
//!                                      padded to ~500 ms so the pipeline has real frames)
//!
//! All routes accept any Content-Type and ignore the request body — they exist
//! only to keep the voicebot pipeline happy during load tests.
//!
//! Usage
//! -----
//!   cargo run -p mock-provider -- --port 8000
//!   MOCK_LATENCY_MS=20 cargo run -p mock-provider
//!
//! Environment variables
//! ---------------------
//!   MOCK_PORT              TCP port to listen on (default 8000)
//!   MOCK_LATENCY_MS        Extra artificial latency per request in ms (default 0)
//!   RUST_LOG               tracing filter (default "info")

use std::env;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::time::sleep;
use tracing::info;

// ── Shared configuration ──────────────────────────────────────────────────────

#[derive(Clone)]
struct Cfg {
    latency: Duration,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build a short audible tone as raw PCM16 LE so the loadtest analyzer can
/// distinguish a real bot response from silence.
fn tone_pcm(num_samples: usize, sample_rate_hz: f32, frequency_hz: f32) -> Vec<u8> {
    let amplitude = i16::MAX as f32 * 0.2;
    let mut pcm = Vec::with_capacity(num_samples * 2);

    for sample_index in 0..num_samples {
        let phase =
            2.0 * std::f32::consts::PI * frequency_hz * sample_index as f32 / sample_rate_hz;
        let sample = (phase.sin() * amplitude) as i16;
        pcm.extend_from_slice(&sample.to_le_bytes());
    }

    pcm
}

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn health() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}

async fn models() -> impl IntoResponse {
    Json(json!({
        "object": "list",
        "data": [
            {"id": "mock-asr",  "object": "model", "owned_by": "mock"},
            {"id": "mock-llm",  "object": "model", "owned_by": "mock"},
            {"id": "mock-tts",  "object": "model", "owned_by": "mock"}
        ]
    }))
}

/// POST /v1/audio/transcriptions
/// Accepts multipart/form-data (ignored) and returns a canned transcript.
async fn transcriptions(State(cfg): State<Cfg>) -> impl IntoResponse {
    if cfg.latency > Duration::ZERO {
        sleep(cfg.latency).await;
    }
    // verbose_json format that SpeachesAsrProvider expects
    Json(json!({
        "task": "transcribe",
        "language": "en",
        "duration": 1.0,
        "text": "hello",
        "segments": [
            {
                "id": 0,
                "seek": 0,
                "start": 0.0,
                "end": 1.0,
                "text": " hello",
                "tokens": [],
                "temperature": 0.0,
                "avg_logprob": -0.3,
                "compression_ratio": 1.0,
                "no_speech_prob": 0.01
            }
        ]
    }))
}

/// POST /v1/chat/completions
/// Returns a streaming SSE chat completion with a short canned reply.
async fn chat_completions(State(cfg): State<Cfg>) -> Response {
    if cfg.latency > Duration::ZERO {
        sleep(cfg.latency).await;
    }

    // Build two delta chunks + [DONE] in one go — small enough to fit in one
    // TCP segment, which is fine for our purposes.
    let id = "chatcmpl-mock";
    let model = "mock-llm";
    let created: u64 = 1_700_000_000;

    let chunks: &[&str] = &["Hello! ", "How can I help you?"];
    let mut body = String::new();

    for (i, text) in chunks.iter().enumerate() {
        let finish = if i + 1 == chunks.len() { "stop" } else { "" };
        let delta = if finish.is_empty() {
            json!({"content": text})
        } else {
            json!({"content": text, "finish_reason": "stop"})
        };
        let chunk: Value = json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": created,
            "model": model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": if finish.is_empty() { Value::Null } else { json!(finish) }}]
        });
        body.push_str("data: ");
        body.push_str(&serde_json::to_string(&chunk).unwrap());
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .body(Body::from(body))
        .unwrap()
}

/// POST /v1/audio/speech
/// Returns raw PCM-16 LE at 16 kHz (500 ms tone).
/// The Content-Type mimics what Speaches returns for `response_format=pcm`.
async fn audio_speech(State(cfg): State<Cfg>) -> Response {
    if cfg.latency > Duration::ZERO {
        sleep(cfg.latency).await;
    }
    // 500 ms at 16 kHz = 8 000 samples = 16 000 bytes
    let pcm = tone_pcm(8_000, 16_000.0, 660.0);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "audio/pcm")
        .body(Body::from(pcm))
        .unwrap()
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let port: u16 = env::var("MOCK_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8000);

    // Also accept --port CLI arg
    let port = env::args()
        .skip(1)
        .collect::<Vec<_>>()
        .windows(2)
        .find(|w| w[0] == "--port")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(port);

    let latency_ms: u64 = env::var("MOCK_LATENCY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let cfg = Cfg {
        latency: Duration::from_millis(latency_ms),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(models))
        .route("/v1/audio/transcriptions", post(transcriptions))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/audio/speech", post(audio_speech))
        .with_state(cfg);

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| {
            eprintln!("FATAL: cannot bind {addr}: {e}");
            std::process::exit(1);
        });

    info!(addr = %addr, latency_ms, "mock-provider listening");

    axum::serve(listener, app).await.expect("server error");
}
