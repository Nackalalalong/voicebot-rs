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

use std::convert::Infallible;
use std::env;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use futures::stream;
use serde_json::json;
use tokio::time::sleep;
use tracing::{error, info};

// ── Shared configuration ──────────────────────────────────────────────────────

const TARGET_SAMPLE_RATE_HZ: u32 = 16_000;
const TTS_FRAME_BYTES: usize = 640;
const TTS_FRAME_DURATION_MS: u64 = 20;
const DEFAULT_TTS_WAV_BYTES: &[u8] = include_bytes!("../../../tests/fixtures/audio/henry.wav");

#[derive(Clone)]
struct Cfg {
    latency: Duration,
    assets: Arc<Assets>,
}

#[derive(Clone)]
struct Assets {
    models_json: Bytes,
    transcription_json: Bytes,
    chat_sse: Bytes,
    tts_pcm: Bytes,
    tts_pcm_frames: Arc<[Bytes]>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn build_response(content_type: &'static str, body: Bytes) -> Response {
    match Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
    {
        Ok(response) => response,
        Err(error) => {
            error!(%error, "failed to build mock-provider response");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn split_pcm_frames(pcm: &Bytes) -> Arc<[Bytes]> {
    pcm.chunks(TTS_FRAME_BYTES)
        .map(Bytes::copy_from_slice)
        .collect::<Vec<_>>()
        .into()
}

fn build_sse_response(body: Bytes) -> Response {
    match Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .body(Body::from(body))
    {
        Ok(response) => response,
        Err(error) => {
            error!(%error, "failed to build mock-provider SSE response");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn maybe_sleep(latency: Duration) -> impl std::future::Future<Output = ()> {
    async move {
        if latency > Duration::ZERO {
            sleep(latency).await;
        }
    }
}

fn encode_pcm_bytes(samples: &[i16]) -> Bytes {
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    Bytes::from(pcm)
}

fn resample_linear(samples: &[i16], input_rate_hz: u32, output_rate_hz: u32) -> Vec<i16> {
    if input_rate_hz == output_rate_hz || samples.is_empty() {
        return samples.to_vec();
    }

    let output_len =
        ((samples.len() as u64) * (output_rate_hz as u64) / (input_rate_hz as u64)) as usize;
    let mut output = Vec::with_capacity(output_len);
    let ratio = input_rate_hz as f64 / output_rate_hz as f64;

    for output_index in 0..output_len {
        let source_position = output_index as f64 * ratio;
        let left_index = source_position.floor() as usize;
        let right_index = (left_index + 1).min(samples.len() - 1);
        let fraction = source_position - left_index as f64;
        let left = samples[left_index] as f64;
        let right = samples[right_index] as f64;
        let interpolated = left + (right - left) * fraction;
        output.push(interpolated.round() as i16);
    }

    output
}

fn load_tts_pcm() -> Result<Bytes, String> {
    let reader = hound::WavReader::new(Cursor::new(DEFAULT_TTS_WAV_BYTES))
        .map_err(|error| format!("failed to read embedded TTS wav: {error}"))?;
    let spec = reader.spec();

    if spec.channels != 1 {
        return Err(format!(
            "embedded TTS wav must be mono, got {} channels",
            spec.channels
        ));
    }
    if spec.bits_per_sample != 16 || spec.sample_format != hound::SampleFormat::Int {
        return Err(format!(
            "embedded TTS wav must be 16-bit PCM, got bits_per_sample={} sample_format={:?}",
            spec.bits_per_sample, spec.sample_format
        ));
    }

    let samples: Vec<i16> = reader
        .into_samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to decode embedded TTS wav samples: {error}"))?;

    let samples = resample_linear(&samples, spec.sample_rate, TARGET_SAMPLE_RATE_HZ);
    Ok(encode_pcm_bytes(&samples))
}

fn load_assets() -> Result<Assets, String> {
    let models_json = Bytes::from_static(
        br#"{"object":"list","data":[{"id":"mock-asr","object":"model","owned_by":"mock"},{"id":"mock-llm","object":"model","owned_by":"mock"},{"id":"mock-tts","object":"model","owned_by":"mock"}]}"#,
    );

    let transcription_json = Bytes::from_static(
        br#"{"task":"transcribe","language":"en","duration":1.0,"text":"hello","segments":[{"id":0,"seek":0,"start":0.0,"end":1.0,"text":" hello","tokens":[],"temperature":0.0,"avg_logprob":-0.3,"compression_ratio":1.0,"no_speech_prob":0.01}]}"#,
    );

    let chat_sse = Bytes::from_static(
        br#"data: {"id":"chatcmpl-mock","object":"chat.completion.chunk","created":1700000000,"model":"mock-llm","choices":[{"index":0,"delta":{"content":"Hello, how can I help you?"},"finish_reason":"stop"}]}

data: [DONE]

"#,
    );

    let tts_pcm = load_tts_pcm()?;
    let tts_pcm_frames = split_pcm_frames(&tts_pcm);

    Ok(Assets {
        models_json,
        transcription_json,
        chat_sse,
        tts_pcm,
        tts_pcm_frames,
    })
}

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn health() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}

async fn models(State(cfg): State<Cfg>) -> Response {
    build_response("application/json", cfg.assets.models_json.clone())
}

/// POST /v1/audio/transcriptions
/// Accepts multipart/form-data (ignored) and returns a canned transcript.
async fn transcriptions(State(cfg): State<Cfg>) -> impl IntoResponse {
    maybe_sleep(cfg.latency).await;
    build_response("application/json", cfg.assets.transcription_json.clone())
}

/// POST /v1/chat/completions
/// Returns a streaming SSE chat completion with a short canned reply.
async fn chat_completions(State(cfg): State<Cfg>) -> Response {
    maybe_sleep(cfg.latency).await;
    build_sse_response(cfg.assets.chat_sse.clone())
}

/// POST /v1/audio/speech
/// Returns raw PCM-16 LE at 16 kHz loaded once from `tests/fixtures/audio/henry.wav`.
/// The Content-Type mimics what Speaches returns for `response_format=pcm`.
async fn audio_speech(State(cfg): State<Cfg>) -> Response {
    maybe_sleep(cfg.latency).await;

    let frames = cfg.assets.tts_pcm_frames.clone();
    let body_stream = stream::unfold((0usize, frames), |(index, frames)| async move {
        if index >= frames.len() {
            return None;
        }

        if index > 0 {
            sleep(Duration::from_millis(TTS_FRAME_DURATION_MS)).await;
        }

        let frame = frames[index].clone();
        Some((Ok::<Bytes, Infallible>(frame), (index + 1, frames)))
    });

    match Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "audio/pcm")
        .body(Body::from_stream(body_stream))
    {
        Ok(response) => response,
        Err(error) => {
            error!(%error, "failed to build mock-provider TTS stream response");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
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

    let assets = load_assets().unwrap_or_else(|error| {
        eprintln!("FATAL: failed to initialize mock-provider assets: {error}");
        std::process::exit(1);
    });

    let tts_bytes = assets.tts_pcm.len();

    let cfg = Cfg {
        latency: Duration::from_millis(latency_ms),
        assets: Arc::new(assets),
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

    info!(
        addr = %addr,
        latency_ms,
        tts_bytes,
        "mock-provider listening"
    );

    axum::serve(listener, app).await.expect("server error");
}
