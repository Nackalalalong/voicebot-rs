use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::audio::AudioFrame;
use common::config::{AppConfig, AsteriskConfig};
use common::events::{PipelineEvent, SessionConfig};
use common::types::{AsrProviderType, Language, LlmProviderType, TtsProviderType};
use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;
use voicebot_core::session::PipelineSession;

use crate::ari_client::AriRestClient;
use crate::audiosocket::{
    frame_to_pcm_bytes, pcm_bytes_to_frame, read_packet, write_audio_packet, write_hangup_packet,
};
use crate::error::AriError;
use crate::events::AriChannel;

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
}

impl AriTransport {
    pub fn new(config: AsteriskConfig, app_config: Arc<AppConfig>) -> Self {
        Self { config, app_config }
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
                        let channel_id = channel.id.clone();
                        let session_cancel = CancellationToken::new();
                        registry
                            .register(channel_id.clone(), session_cancel.clone())
                            .await;

                        let rest = Arc::clone(&rest);
                        let ari_config = self.config.clone();
                        let app_config = Arc::clone(&self.app_config);
                        let registry = Arc::clone(&registry);

                        tokio::spawn(async move {
                            info!(%channel_id, "StasisStart — handling call");
                            if let Err(e) = handle_stasis_start(
                                channel,
                                rest,
                                ari_config,
                                app_config,
                                session_cancel,
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
) -> Result<(), AriError> {
    let channel_id = &channel.id;
    let session_id = Uuid::new_v4();

    // 1. Answer the channel.
    rest.answer_channel(channel_id).await?;

    // 2. Bind an ephemeral TCP port for AudioSocket.
    //    Each call gets its own port — avoids UUID correlation complexity.
    let tcp_listener = tokio::net::TcpListener::bind("0.0.0.0:0").await?;
    let audio_port = tcp_listener.local_addr()?.port();
    let external_host = format!("{}:{}", config.audio_host, audio_port);
    info!(%channel_id, %external_host, "AudioSocket listener bound");

    // 3. Tell Asterisk to connect its audio to our TCP port.
    let ext_chan_id = rest
        .create_external_media(&config.app_name, &external_host, "slin16")
        .await?;

    // 4. Create a mixing bridge and add both channels.
    let bridge_id = rest
        .create_bridge(&format!("voicebot-{}", session_id))
        .await?;
    rest.add_to_bridge(&bridge_id, &[channel_id, &ext_chan_id])
        .await?;

    // 5. Accept the AudioSocket TCP connection (Asterisk connects to us, 10s window).
    let tcp_stream = tokio::select! {
        result = accept_tcp(&tcp_listener) => result?,
        _ = cancel.cancelled() => {
            cleanup(&rest, &bridge_id, channel_id).await;
            return Ok(());
        }
    };
    drop(tcp_listener); // no more connections expected for this call
    info!(%channel_id, %session_id, "AudioSocket connection accepted");

    // 6. Create pipeline I/O channels.
    let (audio_tx, audio_rx) = mpsc::channel::<AudioFrame>(50);
    let (egress_tx, egress_rx) = mpsc::channel::<PipelineEvent>(200);

    // 7. Build session config from app-level defaults.
    let defaults = &app_config.session_defaults;
    let session_config = SessionConfig {
        session_id,
        language: Language::from_str_loose(&defaults.language),
        asr_provider: AsrProviderType::from_str_loose(&defaults.asr_provider),
        tts_provider: TtsProviderType::from_str_loose(&defaults.tts_provider),
        llm_provider: LlmProviderType::from_str_loose(&defaults.llm_provider),
        vad_config: app_config.vad.clone(),
    };

    // 8. Start the pipeline session.
    let mut session =
        match PipelineSession::start_with_config(&app_config, session_config, audio_rx, egress_tx)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                error!(%channel_id, "failed to start pipeline session: {}", e);
                cleanup(&rest, &bridge_id, channel_id).await;
                return Err(AriError::Session(e.to_string()));
            }
        };

    // 9. Bridge AudioSocket ↔ pipeline until hangup, cancel, or SessionEnd.
    run_bridge(
        session_id,
        channel_id,
        tcp_stream,
        audio_tx,
        egress_rx,
        cancel.clone(),
    )
    .await;

    // 10. Tear down.
    session.terminate().await;
    cleanup(&rest, &bridge_id, channel_id).await;

    info!(%channel_id, %session_id, "call ended");
    Ok(())
}

// ---------------------------------------------------------------------------
// AudioSocket bridge loop
// ---------------------------------------------------------------------------

/// Bidirectional bridge between the AudioSocket TCP stream and pipeline channels.
///
/// Inbound  (Asterisk → us):  0x10 audio packets → `audio_tx` as `AudioFrame`.
/// Outbound (us → Asterisk): `TtsAudioChunk` events → 0x10 audio packets.
///
/// Exits on: 0x00 hangup packet, `cancel` fired, `SessionEnd` event, or I/O error.
async fn run_bridge(
    session_id: Uuid,
    channel_id: &str,
    tcp_stream: TcpStream,
    audio_tx: mpsc::Sender<AudioFrame>,
    mut egress_rx: mpsc::Receiver<PipelineEvent>,
    cancel: CancellationToken,
) {
    let (mut read_half, mut write_half) = tokio::io::split(tcp_stream);
    let mut frame_ts_ms: u64 = 0;

    loop {
        tokio::select! {
            biased;

            _ = cancel.cancelled() => {
                let _ = write_hangup_packet(&mut write_half).await;
                break;
            }

            // Read from Asterisk via AudioSocket.
            packet = read_packet(&mut read_half) => {
                match packet {
                    Ok(pkt) => match pkt.kind {
                        0x00 => {
                            // Asterisk sent hangup.
                            info!(%channel_id, "AudioSocket hangup received");
                            cancel.cancel();
                            break;
                        }
                        0x01 => {
                            let uuid = String::from_utf8_lossy(&pkt.payload);
                            info!(%channel_id, audiosocket_uuid = %uuid, "AudioSocket UUID");
                        }
                        0x10 if !pkt.payload.is_empty() => {
                            // slin16 audio frame → pipeline.
                            let samples = pkt.payload.len() / 2;
                            let frame = pcm_bytes_to_frame(&pkt.payload, frame_ts_ms);
                            // Advance timestamp by the number of samples at 16kHz → ms.
                            frame_ts_ms += (samples as u64 * 1000) / 16000;
                            if audio_tx.try_send(frame).is_err() {
                                warn!(%session_id, "audio ingress channel full — dropping frame");
                            }
                        }
                        _ => {} // ignore empty audio or unknown kinds
                    },
                    Err(e) => {
                        warn!(%channel_id, "AudioSocket read error: {}", e);
                        cancel.cancel();
                        break;
                    }
                }
            }

            // Forward TTS audio from pipeline to Asterisk.
            event = egress_rx.recv() => {
                match event {
                    Some(PipelineEvent::TtsAudioChunk { frame, .. }) => {
                        let bytes = frame_to_pcm_bytes(&frame);
                        // Split to 320-byte chunks = 10ms @ 16kHz.
                        for chunk in bytes.chunks(320) {
                            if write_audio_packet(&mut write_half, chunk).await.is_err() {
                                warn!(%channel_id, "AudioSocket write error");
                                cancel.cancel();
                                break;
                            }
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

    // Flush the write half before dropping.
    let _ = write_half.flush().await;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn accept_tcp(listener: &tokio::net::TcpListener) -> Result<TcpStream, AriError> {
    let (stream, addr) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
        .await
        .map_err(|_| AriError::Timeout)??;
    info!(peer = %addr, "AudioSocket TCP connection from Asterisk");
    Ok(stream)
}

async fn cleanup(rest: &AriRestClient, bridge_id: &str, channel_id: &str) {
    if let Err(e) = rest.destroy_bridge(bridge_id).await {
        warn!(bridge_id, "failed to destroy bridge: {}", e);
    }
    if let Err(e) = rest.hangup_channel(channel_id).await {
        warn!(channel_id, "failed to hang up channel: {}", e);
    }
}
