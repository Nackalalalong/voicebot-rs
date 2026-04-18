use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use common::audio::AudioFrame;
use common::config::{AppConfig, AsteriskConfig};
use common::events::{PipelineEvent, SessionConfig};
use common::types::{AsrProviderType, Language, LlmProviderType, TtsProviderType};
use futures::StreamExt;
use async_trait::async_trait;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{sleep_until, Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;
use voicebot_core::session::PipelineSession;

use crate::ari_client::AriRestClient;
use crate::error::AriError;
use crate::events::AriChannel;
use crate::rtp::{build_pcmu_packet, parse_rtp_payload, pcmu_payload_to_frame};

const RTP_FRAME_16K_SAMPLES: usize = 320;
const RTP_TIMESTAMP_STEP: u32 = 160;
const RTP_PACKET_INTERVAL: Duration = Duration::from_millis(20);
const CONVERSATION_MEMORY_FIELD: &str = "conversation_memory";
const CONVERSATION_MEMORY_TTL_SECS: u64 = 3600;

#[derive(Clone)]
struct RedisConversationMemoryBackend {
    redis: cache::RedisPool,
}

impl RedisConversationMemoryBackend {
    fn new(redis: cache::RedisPool) -> Self {
        Self { redis }
    }
}

#[async_trait]
impl voicebot_core::agent::memory::ConversationMemoryBackend for RedisConversationMemoryBackend {
    async fn load(&self, session_id: Uuid) -> Result<Option<Vec<common::types::Message>>, String> {
        let mut redis = self.redis.clone();
        cache::session::get_field(&mut redis, session_id, CONVERSATION_MEMORY_FIELD)
            .await
            .map_err(|error| error.to_string())
    }

    async fn save(&self, session_id: Uuid, messages: &[common::types::Message]) -> Result<(), String> {
        let mut redis = self.redis.clone();
        cache::session::set_field(&mut redis, session_id, CONVERSATION_MEMORY_FIELD, &messages)
            .await
            .map_err(|error| error.to_string())?;
        cache::session::extend_ttl(&mut redis, session_id, CONVERSATION_MEMORY_TTL_SECS)
            .await
            .map_err(|error| error.to_string())
    }

    async fn clear(&self, session_id: Uuid) -> Result<(), String> {
        let mut redis = self.redis.clone();
        cache::session::del_field(&mut redis, session_id, CONVERSATION_MEMORY_FIELD)
            .await
            .map_err(|error| error.to_string())
    }
}

// ---------------------------------------------------------------------------
// Call registry — maps caller channel-id → per-session cancel token.
// ---------------------------------------------------------------------------

struct CallRegistry(Mutex<HashMap<String, CancellationToken>>);

impl CallRegistry {
    fn new() -> Self {
        Self(Mutex::new(HashMap::new()))
    }

    async fn register(&self, channel_id: String, token: CancellationToken) {
        self.0.lock().await.insert(channel_id, token);
    }

    async fn cancel(&self, channel_id: &str) {
        if let Some(token) = self.0.lock().await.get(channel_id) {
            token.cancel();
        }
    }

    async fn unregister(&self, channel_id: &str) {
        self.0.lock().await.remove(channel_id);
    }
}

// ---------------------------------------------------------------------------
// Public Transport handle
// ---------------------------------------------------------------------------

/// Asterisk ARI transport.
///
/// Call `run()` to connect to the ARI WebSocket and process calls.
/// The future resolves when the WebSocket closes or an unrecoverable error occurs.
pub struct AriTransport {
    config: AsteriskConfig,
    app_config: Arc<AppConfig>,
    /// Optional platform context enables phone-number → campaign routing (C7).
    db: Option<db::PgPool>,
    redis: Option<cache::RedisPool>,
}

impl AriTransport {
    pub fn new(config: AsteriskConfig, app_config: Arc<AppConfig>) -> Self {
        Self { config, app_config, db: None, redis: None }
    }

    /// Attach platform DB + Redis so inbound calls can be routed by phone number.
    pub fn with_routing(mut self, db: db::PgPool, redis: cache::RedisPool) -> Self {
        self.db = Some(db);
        self.redis = Some(redis);
        self
    }

    /// Connect to ARI and enter the event loop.
    pub async fn run(self) -> Result<(), AriError> {
        let ws_url = format!(
            "ws://{}:{}/ari/events?api_key={}:{}&app={}",
            self.config.ari_host,
            self.config.ari_port,
            self.config.username,
            self.config.password,
            self.config.app_name,
        );
        info!(url = %ws_url, "connecting to ARI WebSocket");

        let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .map_err(|e| AriError::WebSocket(e.to_string()))?;

        info!("ARI WebSocket connected");

        let (_, mut ws_rx) = ws_stream.split();
        let rest = Arc::new(AriRestClient::new(&self.config));
        let registry = Arc::new(CallRegistry::new());

        while let Some(msg) = ws_rx.next().await {
            use tokio_tungstenite::tungstenite::Message;

            let text = match msg {
                Ok(Message::Text(t)) => t,
                Ok(Message::Close(_)) => {
                    info!("ARI WebSocket closed by server");
                    break;
                }
                Err(e) => {
                    error!("ARI WebSocket error: {}", e);
                    break;
                }
                _ => continue,
            };

            let event: crate::events::AriEvent = match serde_json::from_str(&text) {
                Ok(e) => e,
                Err(e) => {
                    warn!(error = %e, raw = %text, "failed to parse ARI event");
                    continue;
                }
            };

            match event.kind.as_str() {
                "StasisStart" => {
                    if let Some(channel) = event.channel {
                        if channel.name.starts_with("UnicastRTP/") {
                            info!(channel_id = %channel.id, channel_name = %channel.name, "ignoring transport-owned RTP channel");
                            continue;
                        }
                        let channel_id = channel.id.clone();
                        let session_cancel = CancellationToken::new();
                        registry
                            .register(channel_id.clone(), session_cancel.clone())
                            .await;

                        let rest = Arc::clone(&rest);
                        let ari_config = self.config.clone();
                        let app_config = Arc::clone(&self.app_config);
                        let registry = Arc::clone(&registry);
                        let db = self.db.clone();
                        let redis = self.redis.clone();

                        tokio::spawn(async move {
                            info!(%channel_id, "StasisStart — handling call");
                            if let Err(e) = handle_stasis_start(
                                channel,
                                rest,
                                ari_config,
                                app_config,
                                session_cancel,
                                db,
                                redis,
                            )
                            .await
                            {
                                error!(%channel_id, error = %e, "call handler failed");
                            }
                            registry.unregister(&channel_id).await;
                            info!(%channel_id, "call handler finished");
                        });
                    }
                }

                "StasisEnd" | "ChannelHangupRequest" | "ChannelDestroyed" => {
                    if let Some(channel) = event.channel {
                        registry.cancel(&channel.id).await;
                    }
                }

                "ChannelDtmfReceived" => {
                    // # or * cancels the session (DTMF interrupt).
                    if let (Some(channel), Some(digit)) = (event.channel, event.digit) {
                        if matches!(digit.as_str(), "#" | "*") {
                            info!(channel_id = %channel.id, %digit, "DTMF interrupt — cancelling session");
                            registry.cancel(&channel.id).await;
                        }
                    }
                }

                _ => {} // log unknown events at debug level only
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Per-call handler
// ---------------------------------------------------------------------------

async fn handle_stasis_start(
    channel: AriChannel,
    rest: Arc<AriRestClient>,
    config: AsteriskConfig,
    app_config: Arc<AppConfig>,
    cancel: CancellationToken,
    db: Option<db::PgPool>,
    redis: Option<cache::RedisPool>,
) -> Result<(), AriError> {
    let channel_id = &channel.id;
    let session_id = Uuid::new_v4();
    let memory_backend = redis.clone().map(|redis| {
        Arc::new(RedisConversationMemoryBackend::new(redis))
            as Arc<dyn voicebot_core::agent::memory::ConversationMemoryBackend>
    });

    // C7: Resolve tenant/campaign from caller's phone number via Redis routing table.
    let mut tenant_id: Option<Uuid> = None;
    let mut campaign_id: Option<Uuid> = None;
    let mut session_system_prompt: Option<String> = None;
    let mut session_asr: Option<AsrProviderType> = None;
    let mut session_tts: Option<TtsProviderType> = None;

    if let (Some(db_pool), Some(mut redis_pool)) = (db.as_ref(), redis) {
        let caller_number = channel.caller.as_ref().map(|c| c.number.as_str()).unwrap_or("");
        if !caller_number.is_empty() {
            if let Ok(Some(route)) = cache::routing::get_route(&mut redis_pool, caller_number).await {
                tenant_id = Some(route.tenant_id);
                campaign_id = Some(route.campaign_id);
                info!(%session_id, %caller_number, tenant = %route.tenant_id, campaign = %route.campaign_id, "phone routing resolved");

                // Load campaign config from cache or DB.
                let cached = cache::campaign::get_config(&mut redis_pool, route.campaign_id).await.ok().flatten();
                let campaign = if let Some(c) = cached {
                    Some(c)
                } else {
                    db::queries::campaigns::get_by_id(db_pool, route.tenant_id, route.campaign_id).await.ok()
                };
                if let Some(c) = campaign {
                    session_system_prompt = Some(c.system_prompt);
                    session_asr = Some(AsrProviderType::from_str_loose(&c.asr_provider));
                    session_tts = Some(TtsProviderType::from_str_loose(&c.tts_provider));
                }
            }
        }
    }

    // 1. Answer the channel.
    rest.answer_channel(channel_id).await?;

    // 2. Bind an ephemeral UDP port for RTP external media.
    let udp_socket = UdpSocket::bind("0.0.0.0:0").await?;
    let audio_port = udp_socket.local_addr()?.port();
    let external_host = format!("{}:{}", config.audio_host, audio_port);
    info!(%channel_id, %external_host, "RTP socket bound");

    // 3. Tell Asterisk to connect its audio to our UDP port.
    let ext_media = rest
        .create_external_media(&config.app_name, &external_host, "ulaw")
        .await?;
    let ext_media_channel_id = ext_media.id.clone();

    // 4. Create a mixing bridge and add both channels.
    let bridge_id = rest
        .create_bridge(&format!("voicebot-{}", session_id))
        .await?;
    rest.add_to_bridge(&bridge_id, &[channel_id, &ext_media.id])
        .await?;

    // 5. Create pipeline I/O channels.
    let (audio_tx, audio_rx) = mpsc::channel::<AudioFrame>(50);
    let (egress_tx, egress_rx) = mpsc::channel::<PipelineEvent>(200);

    // 6. Build session config from app-level defaults, overridden by campaign routing.
    let defaults = &app_config.session_defaults;
    let session_config = SessionConfig {
        session_id,
        language: Language::from_str_loose(&defaults.language),
        asr_provider: session_asr.unwrap_or_else(|| AsrProviderType::from_str_loose(&defaults.asr_provider)),
        tts_provider: session_tts.unwrap_or_else(|| TtsProviderType::from_str_loose(&defaults.tts_provider)),
        llm_provider: LlmProviderType::from_str_loose(&defaults.llm_provider),
        vad_config: app_config.vad.clone(),
        system_prompt: session_system_prompt,
        tenant_id,
        campaign_id,
    };

    // 7. Start the pipeline session.
    let mut session =
        match PipelineSession::start_with_config_and_memory(&app_config, session_config, audio_rx, egress_tx, memory_backend)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                error!(%channel_id, "failed to start pipeline session: {}", e);
                cleanup(&rest, &bridge_id, channel_id, Some(&ext_media_channel_id)).await;
                return Err(AriError::Session(e.to_string()));
            }
        };
    let reload_handle = match (
        campaign_id,
        session.agent_controller(),
        std::env::var("REDIS_URL").ok(),
    ) {
        (Some(campaign_id), Some(controller), Some(redis_url)) => Some(spawn_campaign_reload_task(
            session_id,
            campaign_id,
            controller,
            redis_url,
        )),
        _ => None,
    };

    // 8. Bridge RTP ↔ pipeline until hangup, cancel, or SessionEnd.
    run_bridge(
        session_id,
        channel_id,
        udp_socket,
        ext_media.remote_addr,
        audio_tx,
        egress_rx,
        cancel.clone(),
    )
    .await;

    if let Some(handle) = reload_handle {
        handle.abort();
        let _ = handle.await;
    }

    // 9. Tear down.
    session.terminate().await;
    cleanup(&rest, &bridge_id, channel_id, Some(&ext_media_channel_id)).await;

    info!(%channel_id, %session_id, "call ended");
    Ok(())
}

// ---------------------------------------------------------------------------
// AudioSocket bridge loop
// ---------------------------------------------------------------------------

/// Bidirectional bridge between RTP packets and pipeline channels.
///
/// Inbound  (Asterisk → us): RTP/PCMU packets → `audio_tx` as 16 kHz `AudioFrame`.
/// Outbound (us → Asterisk): `TtsAudioChunk` events → RTP/PCMU packets.
///
/// Exits on: `cancel` fired, `SessionEnd` event, or I/O error.
async fn run_bridge(
    session_id: Uuid,
    channel_id: &str,
    udp_socket: UdpSocket,
    remote_addr: SocketAddr,
    audio_tx: mpsc::Sender<AudioFrame>,
    mut egress_rx: mpsc::Receiver<PipelineEvent>,
    cancel: CancellationToken,
) {
    let mut frame_ts_ms: u64 = 0;
    let seed = Uuid::new_v4();
    let seed_bytes = seed.as_bytes();
    let mut sequence = u16::from_be_bytes([seed_bytes[0], seed_bytes[1]]);
    let mut timestamp =
        u32::from_be_bytes([seed_bytes[2], seed_bytes[3], seed_bytes[4], seed_bytes[5]]);
    let ssrc = u32::from_be_bytes([seed_bytes[6], seed_bytes[7], seed_bytes[8], seed_bytes[9]]);
    let mut packet_buffer = [0u8; 2048];
    let mut active_remote_addr = remote_addr;
    let mut next_rtp_send_deadline: Option<Instant> = None;

    loop {
        tokio::select! {
            biased;

            _ = cancel.cancelled() => {
                break;
            }

            recv_result = udp_socket.recv_from(&mut packet_buffer) => {
                match recv_result {
                    Ok((received, source_addr)) => {
                        if source_addr != active_remote_addr {
                            let learned_remote_addr = SocketAddr::new(
                                source_addr.ip(),
                                active_remote_addr.port(),
                            );
                            info!(
                                %channel_id,
                                expected_peer = %active_remote_addr,
                                actual_peer = %source_addr,
                                learned_peer = %learned_remote_addr,
                                "learning RTP peer address from live traffic"
                            );
                            active_remote_addr = learned_remote_addr;
                        }
                        let Some(payload) = parse_rtp_payload(&packet_buffer[..received]) else {
                            continue;
                        };
                        if payload.is_empty() {
                            continue;
                        }
                        let frame = pcmu_payload_to_frame(payload, frame_ts_ms);
                        frame_ts_ms += frame.duration_ms();
                        if audio_tx.try_send(frame).is_err() {
                            warn!(%session_id, "audio ingress channel full — dropping frame");
                        }
                    }
                    Err(e) => {
                        warn!(%channel_id, "RTP read error: {}", e);
                        cancel.cancel();
                        break;
                    }
                }
            }

            // Forward TTS audio from pipeline to Asterisk.
            event = egress_rx.recv() => {
                match event {
                    Some(PipelineEvent::TtsAudioChunk { frame, .. }) => {
                        for chunk in frame.data.chunks(RTP_FRAME_16K_SAMPLES) {
                            if let Some(deadline) = next_rtp_send_deadline {
                                let now = Instant::now();
                                if deadline > now {
                                    sleep_until(deadline).await;
                                }
                            }
                            let packet = build_pcmu_packet(chunk, sequence, timestamp, ssrc);
                            if udp_socket.send_to(&packet, active_remote_addr).await.is_err() {
                                warn!(%channel_id, "RTP write error");
                                cancel.cancel();
                                break;
                            }
                            sequence = sequence.wrapping_add(1);
                            timestamp = timestamp.wrapping_add(RTP_TIMESTAMP_STEP);
                            next_rtp_send_deadline = Some(Instant::now() + RTP_PACKET_INTERVAL);
                        }
                    }
                    Some(PipelineEvent::SessionEnd { .. }) | None => {
                        cancel.cancel();
                        break;
                    }
                    _ => {} // ignore other pipeline events
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn cleanup(
    rest: &AriRestClient,
    bridge_id: &str,
    channel_id: &str,
    external_media_channel_id: Option<&str>,
) {
    if let Err(e) = rest.destroy_bridge(bridge_id).await {
        warn!(bridge_id, "failed to destroy bridge: {}", e);
    }
    if let Some(external_media_channel_id) = external_media_channel_id {
        if let Err(e) = rest.hangup_channel(external_media_channel_id).await {
            warn!(
                channel_id = external_media_channel_id,
                "failed to hang up external media channel: {}", e
            );
        }
    }
    if let Err(e) = rest.hangup_channel(channel_id).await {
        warn!(channel_id, "failed to hang up channel: {}", e);
    }
}

fn spawn_campaign_reload_task(
    session_id: Uuid,
    campaign_id: Uuid,
    controller: voicebot_core::session::SessionAgentController,
    redis_url: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut pubsub = match cache::campaign::subscribe_updates(&redis_url, campaign_id).await {
            Ok(pubsub) => pubsub,
            Err(error) => {
                warn!(%session_id, %campaign_id, error = %error, "failed to subscribe to campaign updates");
                return;
            }
        };
        let mut messages = pubsub.on_message();

        while let Some(message) = messages.next().await {
            let payload: String = match message.get_payload() {
                Ok(payload) => payload,
                Err(error) => {
                    warn!(%session_id, %campaign_id, error = %error, "failed to read campaign update payload");
                    continue;
                }
            };
            let update = match cache::campaign::decode_update(&payload) {
                Ok(update) => update,
                Err(error) => {
                    warn!(%session_id, %campaign_id, error = %error, "failed to decode campaign update payload");
                    continue;
                }
            };

            let tools = voicebot_core::agent::tools_from_metrics(&update.custom_metrics).0;
            controller
                .reload_agent_config(Some(update.system_prompt.clone()), tools)
                .await;
            info!(%session_id, %campaign_id, status = %update.status, "campaign config hot-reloaded");
        }
    })
}
