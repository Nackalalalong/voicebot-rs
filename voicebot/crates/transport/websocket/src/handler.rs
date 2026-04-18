use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use common::audio::AudioFrame;
use common::config::AppConfig;
use common::events::{PipelineEvent, SessionConfig, VadConfig};
use common::types::{AsrProviderType, Language, LlmProviderType, TtsProviderType};
use futures::{SinkExt, StreamExt};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;
use voicebot_core::session::PipelineSession;

use crate::error::TransportError;
use crate::protocol::{parse_client_message, ClientMessage, ServerMessage};

// Needed for .sadd()/.srem() on ConnectionManager (redis async commands trait)
#[allow(unused_imports)]
use redis::AsyncCommands;

const PIPELINE_SAMPLE_RATE: u32 = 16_000;
const PIPELINE_FRAME_SAMPLES: usize = 320;
const PIPELINE_FRAME_DURATION_MS: u64 = 20;

struct WebSocketSessionStart {
    pipeline_config: SessionConfig,
    input_sample_rate: u32,
}

struct InboundAudioDecoder {
    next_timestamp_ms: u64,
    output_residual: Vec<i16>,
    mode: InboundAudioMode,
}

enum InboundAudioMode {
    Canonical,
    Resampled(SessionResampler),
}

struct SessionResampler {
    input_chunk_samples: usize,
    input_buffer: Vec<f32>,
    delay_samples_to_trim: usize,
    resampler: Fft<f32>,
}

impl InboundAudioDecoder {
    fn new(input_sample_rate: u32) -> Result<Self, TransportError> {
        if input_sample_rate == 0 {
            return Err(TransportError::InvalidSampleRate(input_sample_rate));
        }

        let mode = if input_sample_rate == PIPELINE_SAMPLE_RATE {
            InboundAudioMode::Canonical
        } else {
            InboundAudioMode::Resampled(SessionResampler::new(input_sample_rate)?)
        };

        Ok(Self {
            next_timestamp_ms: 0,
            output_residual: Vec::with_capacity(PIPELINE_FRAME_SAMPLES * 2),
            mode,
        })
    }

    fn decode(&mut self, bytes: &[u8]) -> Result<Vec<AudioFrame>, TransportError> {
        if bytes.len() % 2 != 0 {
            return Err(TransportError::InvalidFrameSize(bytes.len()));
        }

        let samples: Vec<i16> = bytes
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();

        let normalized_samples = match &mut self.mode {
            InboundAudioMode::Canonical => samples,
            InboundAudioMode::Resampled(resampler) => resampler.process(&samples)?,
        };

        self.output_residual.extend(normalized_samples);

        let mut frames = Vec::new();
        while self.output_residual.len() >= PIPELINE_FRAME_SAMPLES {
            let frame_samples: Vec<i16> = self
                .output_residual
                .drain(..PIPELINE_FRAME_SAMPLES)
                .collect();
            frames.push(AudioFrame::new(frame_samples, self.next_timestamp_ms));
            self.next_timestamp_ms += PIPELINE_FRAME_DURATION_MS;
        }

        Ok(frames)
    }
}

impl SessionResampler {
    fn new(input_sample_rate: u32) -> Result<Self, TransportError> {
        let input_chunk_samples = samples_per_20ms(input_sample_rate);
        let resampler = Fft::<f32>::new(
            input_sample_rate as usize,
            PIPELINE_SAMPLE_RATE as usize,
            input_chunk_samples,
            1,
            1,
            FixedSync::Input,
        )
        .map_err(|e| TransportError::AudioResampler(e.to_string()))?;
        let delay_samples_to_trim = resampler.output_delay();

        Ok(Self {
            input_chunk_samples,
            input_buffer: Vec::with_capacity(input_chunk_samples * 2),
            delay_samples_to_trim,
            resampler,
        })
    }

    fn process(&mut self, input_samples: &[i16]) -> Result<Vec<i16>, TransportError> {
        self.input_buffer
            .extend(input_samples.iter().map(|&sample| i16_to_f32(sample)));

        let mut output = Vec::new();
        while self.input_buffer.len() >= self.input_chunk_samples {
            let chunk: Vec<f32> = self
                .input_buffer
                .drain(..self.input_chunk_samples)
                .collect();
            let input = InterleavedSlice::<&[f32]>::new(&chunk, 1, self.input_chunk_samples)
                .map_err(|e| TransportError::AudioResampler(e.to_string()))?;
            let resampled = self
                .resampler
                .process(&input, 0, None)
                .map_err(|e| TransportError::AudioResampler(e.to_string()))?
                .take_data();

            let trimmed = if self.delay_samples_to_trim == 0 {
                resampled
            } else {
                let to_trim = self.delay_samples_to_trim.min(resampled.len());
                self.delay_samples_to_trim -= to_trim;
                resampled.into_iter().skip(to_trim).collect()
            };

            output.extend(trimmed.into_iter().map(f32_to_i16));
        }

        Ok(output)
    }
}

fn samples_per_20ms(sample_rate: u32) -> usize {
    ((sample_rate as usize) + 25) / 50
}

fn i16_to_f32(sample: i16) -> f32 {
    sample as f32 / i16::MAX as f32
}

fn f32_to_i16(sample: f32) -> i16 {
    let clamped = sample.clamp(-1.0, 1.0);
    if clamped < 0.0 {
        (clamped * 32768.0) as i16
    } else {
        (clamped * 32767.0) as i16
    }
}

/// Build the Axum router with the `/session` WebSocket endpoint.
/// When called without config, uses stub providers.
pub fn router() -> Router {
    Router::new().route("/session", get(ws_handler_stubs))
}

/// Build the Axum router with config-driven providers.
pub fn router_with_config(config: Arc<AppConfig>) -> Router {
    Router::new()
        .route("/session", get(ws_handler_configured))
        .with_state(config)
}

/// Platform context — optional DB, Redis, JWT, and S3 storage for the authenticated path.
/// Cloned cheaply: PgPool and RedisPool are internally Arc-wrapped.
#[derive(Clone)]
pub struct PlatformContext {
    pub config: Arc<AppConfig>,
    pub db: db::PgPool,
    pub redis: cache::RedisPool,
    pub jwt_secret: String,
    /// Optional S3-compatible storage for call recordings.
    pub storage: Option<storage::StorageClient>,
}

/// State tuple used by the platform-aware Axum handler.
type PlatformState = (Arc<AppConfig>, Arc<PlatformContext>);

/// Build the Axum router with full platform support (JWT auth, CDR, Redis tracking).
pub fn router_with_platform(config: Arc<AppConfig>, platform: Arc<PlatformContext>) -> Router {
    Router::new()
        .route("/session", get(ws_handler_platform))
        .with_state((config, platform))
}

/// Axum handler using stub providers (for testing).
async fn ws_handler_stubs(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_connection(socket, None))
}

/// Axum handler using real providers from config.
async fn ws_handler_configured(
    State(config): State<Arc<AppConfig>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_connection(socket, Some(config)))
}

/// Axum handler with platform support: JWT auth, campaign resolution, CDR, Redis tracking.
async fn ws_handler_platform(
    State((config, platform)): State<PlatformState>,
    Query(params): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Extract and validate JWT from ?token=
    let token = params.get("token").cloned().unwrap_or_default();
    let claims = match auth::validate_token(&platform.jwt_secret, &token) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "ws_handler_platform: JWT validation failed");
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        }
    };
    let tenant_id = match claims.tenant_id() {
        Ok(id) => id,
        Err(e) => {
            tracing::warn!(error = %e, "ws_handler_platform: invalid tenant_id in claims");
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        }
    };

    // Optional campaign_id from query string
    let campaign_id = params
        .get("campaign_id")
        .and_then(|s| Uuid::parse_str(s).ok());

    ws.on_upgrade(move |socket| {
        handle_connection_platform(socket, config, platform, tenant_id, campaign_id)
    })
}

/// Main connection handler. Manages the full lifecycle of a single session:
/// 1. Generate a unique session ID
/// 2. Wait for a `session_start` JSON message from the client (10s timeout)
/// 3. Spawn the core pipeline (VAD → ASR → Agent → TTS)
/// 4. Bridge WebSocket frames ↔ pipeline events until disconnect
/// 5. Terminate the pipeline on exit
async fn handle_connection(ws: WebSocket, app_config: Option<Arc<AppConfig>>) {
    // Transport layer owns the session UUID — core never generates it
    let session_id = Uuid::new_v4();
    tracing::info!(%session_id, "new WebSocket connection");

    let (mut ws_sink, mut ws_stream) = ws.split();
    let vad_config = app_config
        .as_ref()
        .map(|cfg| cfg.vad.clone())
        .unwrap_or_default();
    let channel_config = app_config
        .as_ref()
        .map(|cfg| cfg.channels.clone())
        .unwrap_or_default();

    // Wait for session_start with 10s timeout
    let session_start = match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        wait_for_session_start(&mut ws_stream, session_id, vad_config),
    )
    .await
    {
        Ok(Ok(config)) => config,
        Ok(Err(e)) => {
            tracing::error!(%session_id, "invalid session_start: {}", e);
            return;
        }
        Err(_) => {
            tracing::error!(%session_id, "session_start timeout");
            return;
        }
    };

    // Bounded channels between transport and pipeline:
    // - audio_tx/rx: client PCM audio → VAD/ASR fanout
    // - egress_tx/rx: pipeline events → client WS frames
    // Use configured capacities so load/perf runs can absorb the initial
    // utterance burst without silently losing the first turn.
    let (audio_tx, audio_rx) =
        tokio::sync::mpsc::channel::<AudioFrame>(channel_config.audio_ingress_capacity);
    let (egress_tx, mut egress_rx) =
        tokio::sync::mpsc::channel::<PipelineEvent>(channel_config.event_bus_capacity);

    // Start pipeline session
    let session_result = match app_config {
        Some(ref cfg) => {
            PipelineSession::start_with_config(
                cfg,
                session_start.pipeline_config,
                audio_rx,
                egress_tx,
            )
            .await
        }
        None => {
            PipelineSession::start_with_stubs(session_start.pipeline_config, audio_rx, egress_tx)
                .await
        }
    };
    let mut session = match session_result {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(%session_id, "failed to start session: {}", e);
            return;
        }
    };

    let session_ready = match serde_json::to_string(&ServerMessage::SessionReady) {
        Ok(json) => json,
        Err(error) => {
            tracing::error!(%session_id, %error, "failed to encode session_ready");
            session.terminate().await;
            return;
        }
    };
    if let Err(error) = ws_sink.send(Message::Text(session_ready.into())).await {
        tracing::error!(%session_id, %error, "failed to send session_ready");
        session.terminate().await;
        return;
    }
    tracing::info!(%session_id, "session_ready sent");

    // Run bidirectional bridge
    run_ws_bridge(
        session_id,
        session_start.input_sample_rate,
        &audio_tx,
        &mut egress_rx,
        &mut ws_sink,
        &mut ws_stream,
        None,
    )
    .await;

    // Cleanup
    session.terminate().await;
    tracing::info!(%session_id, "WebSocket session ended");
}

/// Platform connection handler: JWT-authenticated, campaign-aware, CDR-tracked.
async fn handle_connection_platform(
    ws: WebSocket,
    app_config: Arc<AppConfig>,
    platform: Arc<PlatformContext>,
    tenant_id: Uuid,
    campaign_id: Option<Uuid>,
) {
    let session_id = Uuid::new_v4();
    tracing::info!(%session_id, %tenant_id, ?campaign_id, "new platform WebSocket connection");

    let (mut ws_sink, mut ws_stream) = ws.split();
    let vad_config = app_config.vad.clone();
    let channel_config = app_config.channels.clone();

    // Resolve campaign config: Redis → PG fallback
    let mut session_system_prompt: Option<String> = None;
    let mut session_asr: Option<AsrProviderType> = None;
    let mut session_tts: Option<TtsProviderType> = None;
    let mut recording_enabled = false;
    let mut campaign_custom_metrics = serde_json::Value::Null;

    if let Some(cid) = campaign_id {
        let mut redis = platform.redis.clone();
        // Try Redis cache first
        let cached: Option<db::models::Campaign> =
            cache::campaign::get_config(&mut redis, cid).await.ok().flatten();
        let campaign = if let Some(c) = cached {
            Some(c)
        } else {
            // Cache miss — fetch from PG
            match db::queries::campaigns::get_by_id(&platform.db, tenant_id, cid).await {
                Ok(c) => {
                    // Back-fill the cache (best-effort)
                    let _ = cache::campaign::set_config(&mut redis, cid, &c).await;
                    Some(c)
                }
                Err(e) => {
                    tracing::warn!(%session_id, %cid, error = %e, "campaign not found, proceeding without campaign config");
                    None
                }
            }
        };
        if let Some(c) = campaign {
            session_system_prompt = Some(c.system_prompt);
            session_asr = Some(AsrProviderType::from_str_loose(&c.asr_provider));
            session_tts = Some(TtsProviderType::from_str_loose(&c.tts_provider));
            recording_enabled = c.recording_enabled;
            campaign_custom_metrics = c.custom_metrics;
        }
    }

    // Build auto-generated tools from campaign custom_metrics (C5).
    let (campaign_tools, captured_metrics): (
        Vec<Box<dyn voicebot_core::agent::tool::Tool>>,
        std::sync::Arc<tokio::sync::Mutex<serde_json::Map<String, serde_json::Value>>>,
    ) = voicebot_core::agent::tools_from_metrics(&campaign_custom_metrics);

    // Track active session in Redis + enforce max_concurrent_sessions limit.
    let mut redis = platform.redis.clone();
    let active_key = format!("tenant:{}:active_sessions", tenant_id);

    // Fetch the tenant concurrently with the concurrent-limit check.
    let max_concurrent = match db::queries::tenants::get_by_id(&platform.db, tenant_id).await {
        Ok(t) => t.max_concurrent_sessions as i64,
        Err(e) => {
            tracing::warn!(%session_id, %tenant_id, error = %e, "failed to fetch tenant; using default limit 10");
            10
        }
    };
    let active_count: i64 = redis.scard::<_, i64>(&active_key).await.unwrap_or(0);
    if active_count >= max_concurrent {
        tracing::warn!(
            %session_id, %tenant_id,
            active = active_count, limit = max_concurrent,
            "concurrent session limit reached — rejecting new session"
        );
        return; // WS upgrade already happened; transport will close the socket.
    }

    let _ = redis.sadd::<_, _, ()>(&active_key, session_id.to_string()).await;

    // Wait for session_start with 10s timeout
    let session_start = match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        wait_for_session_start(&mut ws_stream, session_id, vad_config),
    )
    .await
    {
        Ok(Ok(mut s)) => {
            // Apply campaign overrides
            s.pipeline_config.tenant_id = Some(tenant_id);
            s.pipeline_config.campaign_id = campaign_id;
            if let Some(prompt) = session_system_prompt {
                s.pipeline_config.system_prompt = Some(prompt);
            }
            if let Some(asr) = session_asr {
                s.pipeline_config.asr_provider = asr;
            }
            if let Some(tts) = session_tts {
                s.pipeline_config.tts_provider = tts;
            }
            s
        }
        Ok(Err(e)) => {
            tracing::error!(%session_id, "invalid session_start: {}", e);
            cleanup_active_session(&platform.redis, tenant_id, session_id).await;
            return;
        }
        Err(_) => {
            tracing::error!(%session_id, "session_start timeout");
            cleanup_active_session(&platform.redis, tenant_id, session_id).await;
            return;
        }
    };

    // Create CDR in DB
    let cdr_req = db::queries::call_records::CreateCallRecord {
        tenant_id,
        campaign_id,
        contact_id: None,
        session_id: &session_id.to_string(),
        direction: "inbound",
        phone_number: "websocket",
    };
    let _ = db::queries::call_records::create(&platform.db, cdr_req).await;

    let (audio_tx, audio_rx) =
        tokio::sync::mpsc::channel::<AudioFrame>(channel_config.audio_ingress_capacity);
    let (egress_tx, mut egress_rx) =
        tokio::sync::mpsc::channel::<PipelineEvent>(channel_config.event_bus_capacity);

    let session_result =
        PipelineSession::start_with_config_and_tools(&app_config, session_start.pipeline_config, audio_rx, egress_tx, campaign_tools).await;
    let mut session = match session_result {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(%session_id, "failed to start session: {}", e);
            cleanup_active_session(&platform.redis, tenant_id, session_id).await;
            return;
        }
    };

    let session_ready = match serde_json::to_string(&ServerMessage::SessionReady) {
        Ok(json) => json,
        Err(error) => {
            tracing::error!(%session_id, %error, "failed to encode session_ready");
            session.terminate().await;
            cleanup_active_session(&platform.redis, tenant_id, session_id).await;
            return;
        }
    };
    if let Err(error) = ws_sink.send(Message::Text(session_ready.into())).await {
        tracing::error!(%session_id, %error, "failed to send session_ready");
        session.terminate().await;
        cleanup_active_session(&platform.redis, tenant_id, session_id).await;
        return;
    }

    let mut recording_samples: Option<Vec<i16>> = if recording_enabled {
        Some(Vec::with_capacity(16_000 * 60)) // pre-alloc 60s
    } else {
        None
    };

    run_ws_bridge(
        session_id,
        session_start.input_sample_rate,
        &audio_tx,
        &mut egress_rx,
        &mut ws_sink,
        &mut ws_stream,
        recording_samples.as_mut(),
    )
    .await;

    // Finalize
    let stats = session.terminate().await;
    cleanup_active_session(&platform.redis, tenant_id, session_id).await;

    // C9: Upload recording to S3 if enabled
    let recording_url = if let (Some(samples), Some(storage)) = (recording_samples.as_ref(), platform.storage.as_ref()) {
        if !samples.is_empty() {
            let wav_bytes = encode_pcm_to_wav(samples, PIPELINE_SAMPLE_RATE);
            let key = storage::StorageClient::recording_key(&tenant_id.to_string(), &session_id.to_string());
            match storage.upload(&key, wav_bytes, "audio/wav").await {
                Ok(_) => {
                    tracing::info!(%session_id, %key, "recording uploaded to S3");
                    Some(key)
                }
                Err(e) => {
                    tracing::warn!(%session_id, error = %e, "failed to upload recording");
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Finalize CDR — merge captured metrics from auto-generated tools (C5).
    let mut custom_metrics_map = captured_metrics.lock().await.clone();
    custom_metrics_map.insert("turn_count".into(), stats.turn_count.into());
    custom_metrics_map.insert("interrupt_count".into(), stats.interrupt_count.into());
    let custom_metrics = serde_json::Value::Object(custom_metrics_map);
    let _ = db::queries::call_records::finalize(
        &platform.db,
        tenant_id,
        &session_id.to_string(),
        "completed",
        Some(stats.duration_secs() as i32),
        recording_url.as_deref(),
        None,
        custom_metrics,
    )
    .await;

    // C10: Write usage record
    {
        use chrono::Utc;
        let ended_at = Utc::now();
        let duration_secs = stats.duration_secs() as i64;
        let started_at = ended_at - chrono::Duration::seconds(duration_secs);
        let _ = db::queries::usage::record_call(
            &platform.db,
            tenant_id,
            campaign_id,
            started_at,
            ended_at,
            duration_secs,
            duration_secs, // asr_seconds ≈ call duration
            0,             // tts_characters (not tracked per-session yet)
            0,             // llm_tokens (not tracked per-session yet)
            0,             // cost_usd_cents (billing not implemented)
        )
        .await;
    }

    tracing::info!(%session_id, "platform WebSocket session ended");
}

/// Encode raw 16-bit mono PCM samples at `sample_rate` Hz into a WAV byte buffer.
fn encode_pcm_to_wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let num_samples = samples.len() as u32;
    let byte_rate = sample_rate * 2; // 1 channel * 16-bit
    let data_bytes = num_samples * 2;
    let total_size = 36 + data_bytes;

    let mut out = Vec::with_capacity((total_size + 8) as usize);
    // RIFF header
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&total_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    // fmt chunk
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    out.extend_from_slice(&1u16.to_le_bytes());  // PCM
    out.extend_from_slice(&1u16.to_le_bytes());  // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());  // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    // data chunk
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Remove a session from the tenant's active-session set in Redis.
async fn cleanup_active_session(redis: &cache::RedisPool, tenant_id: Uuid, session_id: Uuid) {
    let mut conn = redis.clone();
    let key = format!("tenant:{}:active_sessions", tenant_id);
    let _ = conn.srem::<_, _, ()>(&key, session_id.to_string()).await;
}

/// Read WS text frames until we receive a valid `session_start` message.
/// Returns the parsed `SessionConfig`. Caller is responsible for the timeout.
async fn wait_for_session_start(
    ws_stream: &mut futures::stream::SplitStream<WebSocket>,
    session_id: Uuid,
    vad_config: VadConfig,
) -> Result<WebSocketSessionStart, TransportError> {
    while let Some(msg) = ws_stream.next().await {
        match msg {
            Ok(Message::Text(text)) => match parse_client_message(&text) {
                Ok(ClientMessage::SessionStart {
                    language,
                    asr,
                    tts,
                    sample_rate,
                }) => {
                    let input_sample_rate = sample_rate.unwrap_or(PIPELINE_SAMPLE_RATE);
                    if input_sample_rate == 0 {
                        return Err(TransportError::InvalidSampleRate(input_sample_rate));
                    }

                    tracing::info!(
                        %session_id,
                        %language,
                        %asr,
                        %tts,
                        input_sample_rate,
                        "session_start received"
                    );
                    return Ok(WebSocketSessionStart {
                        pipeline_config: SessionConfig {
                            session_id,
                            language: Language::from_str_loose(&language),
                            asr_provider: AsrProviderType::from_str_loose(&asr),
                            tts_provider: TtsProviderType::from_str_loose(&tts),
                            llm_provider: LlmProviderType::OpenAi,
                            vad_config,
                            system_prompt: None,
                            tenant_id: None,
                            campaign_id: None,
                        },
                        input_sample_rate,
                    });
                }
                Ok(_) => {
                    tracing::warn!(%session_id, "expected session_start, got other message");
                }
                Err(e) => return Err(e),
            },
            Ok(Message::Close(_)) => {
                return Err(TransportError::Session(
                    "connection closed before session_start".into(),
                ));
            }
            Err(e) => return Err(TransportError::WebSocket(e)),
            _ => {} // ignore binary/ping/pong while waiting
        }
    }
    Err(TransportError::Session(
        "stream ended before session_start".into(),
    ))
}

/// Bidirectional bridge between the WebSocket and pipeline channels.
///
/// Inbound (client → pipeline): binary frames are parsed as PCM audio and
/// forwarded via `audio_tx`; text frames are parsed as JSON control messages.
///
/// Outbound (pipeline → client): `PipelineEvent`s are translated to JSON
/// text frames or binary audio frames and sent via `ws_sink`.
///
/// The loop exits on client disconnect, `session_end`, or send failure.
/// If `recording_buf` is `Some`, decoded PCM samples are accumulated for
/// optional post-session upload to S3.
async fn run_ws_bridge(
    session_id: Uuid,
    input_sample_rate: u32,
    audio_tx: &tokio::sync::mpsc::Sender<AudioFrame>,
    egress_rx: &mut tokio::sync::mpsc::Receiver<PipelineEvent>,
    ws_sink: &mut futures::stream::SplitSink<WebSocket, Message>,
    ws_stream: &mut futures::stream::SplitStream<WebSocket>,
    mut recording_buf: Option<&mut Vec<i16>>,
) {
    let mut decoder = match InboundAudioDecoder::new(input_sample_rate) {
        Ok(decoder) => decoder,
        Err(e) => {
            tracing::error!(%session_id, "failed to initialize audio decoder: {}", e);
            return;
        }
    };

    if input_sample_rate != PIPELINE_SAMPLE_RATE {
        tracing::info!(%session_id, input_sample_rate, "WebSocket ingress resampling enabled");
    }

    loop {
        tokio::select! {
            msg = ws_stream.next() => {
                match msg {
                    Some(Ok(Message::Binary(bytes))) => {
                        match decoder.decode(&bytes) {
                            Ok(frames) => {
                                for frame in frames {
                                    if let Some(ref mut buf) = recording_buf {
                                        buf.extend_from_slice(&frame.data);
                                    }
                                    if audio_tx.send(frame).await.is_err() {
                                        tracing::debug!(%session_id, "audio channel closed, stopping websocket bridge");
                                        return;
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(%session_id, "invalid audio frame: {}", e);
                            }
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        match parse_client_message(&text) {
                            Ok(ClientMessage::SessionEnd) => {
                                tracing::info!(%session_id, "client requested session end");
                                break;
                            }
                            Ok(_) => {}
                            Err(e) => {
                                tracing::warn!(%session_id, "invalid client message: {}", e);
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(e)) => {
                        tracing::error!(%session_id, "WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
            event = egress_rx.recv() => {
                match event {
                    Some(ev) => {
                        if let Err(e) = send_pipeline_event(ws_sink, ev).await {
                            tracing::error!(%session_id, "failed to send to client: {}", e);
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }
}

/// Translate a `PipelineEvent` into a WebSocket frame and send it.
/// - VAD speech-start notifications → JSON text frames
/// - Transcripts & agent responses → JSON text frames
/// - TTS audio chunks → binary PCM frames
/// - Component errors → JSON error frames
/// - Other internal events (Cancel, Flush, Replace, etc.) are silently dropped.
async fn send_pipeline_event(
    sink: &mut futures::stream::SplitSink<WebSocket, Message>,
    event: PipelineEvent,
) -> Result<(), TransportError> {
    match event {
        PipelineEvent::SpeechStarted { timestamp_ms } => {
            let msg = ServerMessage::SpeechStarted { timestamp_ms };
            let json = serde_json::to_string(&msg)
                .map_err(|e| TransportError::InvalidJson(e.to_string()))?;
            sink.send(Message::Text(json.into())).await?;
        }
        PipelineEvent::PartialTranscript { text, .. } => {
            let msg = ServerMessage::TranscriptPartial { text };
            let json = serde_json::to_string(&msg)
                .map_err(|e| TransportError::InvalidJson(e.to_string()))?;
            sink.send(Message::Text(json.into())).await?;
        }
        PipelineEvent::FinalTranscript { text, .. } => {
            let msg = ServerMessage::TranscriptFinal { text };
            let json = serde_json::to_string(&msg)
                .map_err(|e| TransportError::InvalidJson(e.to_string()))?;
            sink.send(Message::Text(json.into())).await?;
        }
        PipelineEvent::AgentPartialResponse { text } => {
            let msg = ServerMessage::AgentPartial { text };
            let json = serde_json::to_string(&msg)
                .map_err(|e| TransportError::InvalidJson(e.to_string()))?;
            sink.send(Message::Text(json.into())).await?;
        }
        PipelineEvent::AgentFinalResponse { text, .. } => {
            let msg = ServerMessage::AgentFinal { text };
            let json = serde_json::to_string(&msg)
                .map_err(|e| TransportError::InvalidJson(e.to_string()))?;
            sink.send(Message::Text(json.into())).await?;
        }
        PipelineEvent::TtsAudioChunk { frame, .. } => {
            let bytes = frame.to_pcm_bytes();
            sink.send(Message::Binary(bytes.into())).await?;
        }
        PipelineEvent::TtsComplete => {
            let json = serde_json::to_string(&ServerMessage::TtsComplete)
                .map_err(|e| TransportError::InvalidJson(e.to_string()))?;
            sink.send(Message::Text(json.into())).await?;
        }
        PipelineEvent::ComponentError {
            component,
            error,
            recoverable,
        } => {
            let msg = ServerMessage::Error {
                code: format!("{}_{}", component, error),
                recoverable,
            };
            let json = serde_json::to_string(&msg)
                .map_err(|e| TransportError::InvalidJson(e.to_string()))?;
            sink.send(Message::Text(json.into())).await?;
        }
        _ => {} // Internal events not forwarded
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inbound_audio_decoder_passthrough() {
        let mut decoder = InboundAudioDecoder::new(PIPELINE_SAMPLE_RATE).unwrap();
        let samples: Vec<i16> = vec![100, -100, 200];
        let mut input = Vec::new();
        while input.len() < PIPELINE_FRAME_SAMPLES {
            input.extend_from_slice(&samples);
        }
        input.truncate(PIPELINE_FRAME_SAMPLES);

        let bytes: Vec<u8> = input.iter().flat_map(|s| s.to_le_bytes()).collect();
        let frames = decoder.decode(&bytes).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].sample_rate, PIPELINE_SAMPLE_RATE);
        assert_eq!(frames[0].num_samples(), PIPELINE_FRAME_SAMPLES);
    }

    #[test]
    fn test_inbound_audio_decoder_rejects_odd_bytes() {
        let mut decoder = InboundAudioDecoder::new(PIPELINE_SAMPLE_RATE).unwrap();
        let bytes = vec![0u8, 1, 2]; // 3 bytes = odd
        assert!(decoder.decode(&bytes).is_err());
    }

    #[test]
    fn test_inbound_audio_decoder_resamples_48khz() {
        let mut decoder = InboundAudioDecoder::new(48_000).unwrap();
        let silent_frame = vec![0i16; samples_per_20ms(48_000)];
        let bytes: Vec<u8> = silent_frame.iter().flat_map(|s| s.to_le_bytes()).collect();

        let mut frames = Vec::new();
        for _ in 0..8 {
            frames.extend(decoder.decode(&bytes).unwrap());
            if !frames.is_empty() {
                break;
            }
        }

        assert!(!frames.is_empty());
        assert_eq!(frames[0].sample_rate, PIPELINE_SAMPLE_RATE);
        assert_eq!(frames[0].num_samples(), PIPELINE_FRAME_SAMPLES);
    }
}
