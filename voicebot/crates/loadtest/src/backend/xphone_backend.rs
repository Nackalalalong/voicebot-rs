use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tracing::{debug, info};
use xphone::{Call, CallState, Codec, Config, DialOptions, Phone};

use crate::audio::{downsample_16k_to_8k, upsample_8k_to_16k};
use crate::backend::{Phase1Backend, Phase1CallRequest, Phase1CallResult, Phase1InboundRequest};
use crate::config::XphoneBackendConfig;
use crate::error::LoadtestError;

/// SIP virtual phone backend using `xphone`.
///
/// Registers with an Asterisk (or any SIP registrar) as a real SIP phone
/// and places outbound calls or receives inbound calls via native SIP/RTP.
pub struct XphoneBackend {
    phone: Arc<Phone>,
    config: XphoneBackendConfig,
    /// Channel that receives inbound calls from the `on_incoming` callback.
    incoming_rx: crossbeam_channel::Receiver<Arc<Call>>,
}

impl XphoneBackend {
    /// Create and register the phone. Blocks (via `spawn_blocking`) until
    /// registration succeeds or the timeout elapses.
    pub async fn connect(config: XphoneBackendConfig) -> Result<Self, LoadtestError> {
        let xphone_config = Config {
            username: config.username.clone(),
            password: config.password.clone(),
            host: config.sip_host.clone(),
            local_ip: config.local_ip.clone(),
            port: config.sip_port,
            transport: config.transport.clone(),
            rtp_port_min: config.rtp_port_min,
            rtp_port_max: config.rtp_port_max,
            codec_prefs: vec![Codec::PCMU],
            ..Config::default()
        };

        debug!(
            sip_host = %xphone_config.host,
            local_ip = %xphone_config.local_ip,
            rtp_port_min = xphone_config.rtp_port_min,
            rtp_port_max = xphone_config.rtp_port_max,
            "xphone: building phone config"
        );

        let timeout_ms = config.register_timeout_ms;

        // Channel for inbound calls — buffered to avoid blocking the SIP thread.
        let (incoming_tx, incoming_rx) = crossbeam_channel::bounded::<Arc<Call>>(64);

        let phone = tokio::task::spawn_blocking(move || -> Result<Phone, LoadtestError> {
            let phone = Phone::new(xphone_config);

            let (reg_tx, reg_rx) = crossbeam_channel::bounded(1);
            phone.on_registered(move || {
                let _ = reg_tx.try_send(());
            });

            // Forward inbound INVITEs to the channel.
            phone.on_incoming(move |call: Arc<Call>| {
                let _ = incoming_tx.try_send(call);
            });

            phone
                .connect()
                .map_err(|e| LoadtestError::Protocol(format!("SIP connect failed: {e}")))?;

            reg_rx
                .recv_timeout(Duration::from_millis(timeout_ms))
                .map_err(|_| {
                    LoadtestError::Timeout(format!(
                        "SIP registration timed out after {timeout_ms}ms"
                    ))
                })?;

            info!("xphone: registered with SIP server");
            Ok(phone)
        })
        .await??;

        Ok(Self {
            phone: Arc::new(phone),
            config,
            incoming_rx,
        })
    }
}

#[async_trait]
impl Phase1Backend for XphoneBackend {
    async fn run_single_outbound_call(
        &self,
        request: Phase1CallRequest,
    ) -> Result<Phase1CallResult, LoadtestError> {
        let phone = Arc::clone(&self.phone);
        let call_timeout_ms = self.config.call_timeout_ms;

        let result =
            tokio::task::spawn_blocking(move || -> Result<Phase1CallResult, LoadtestError> {
                let started_at = Instant::now();

                let opts = DialOptions {
                    timeout: Duration::from_millis(call_timeout_ms),
                    ..Default::default()
                };

                let call = phone
                    .dial(&request.target_endpoint, opts)
                    .map_err(|e| LoadtestError::Protocol(format!("xphone dial failed: {e}")))?;

                // Wait for the call to become active.
                wait_for_active(&call, Duration::from_millis(call_timeout_ms))?;

                debug!(
                    remote_ip = %call.remote_ip(),
                    remote_port = call.remote_port(),
                    media_active = call.media_session_active(),
                    remote_sdp = %call.remote_sdp().replace('\r', "").replace('\n', " | "),
                    "xphone: outbound media negotiated"
                );

                let connect_ms = started_at.elapsed().as_millis() as u64;

                // Set up ended notification.
                let (ended_tx, ended_rx) = crossbeam_channel::bounded::<()>(1);
                call.on_ended(move |reason| {
                    debug!(?reason, "xphone: call ended");
                    let _ = ended_tx.try_send(());
                });

                // Settle delay.
                if request.settle_before_playback_ms > 0 {
                    std::thread::sleep(Duration::from_millis(request.settle_before_playback_ms));
                }

                // Start RX reader in a background thread.
                let pcm_rx = call.pcm_reader().ok_or_else(|| {
                    LoadtestError::Protocol("xphone: pcm_reader() returned None".into())
                })?;
                let (rx_done_tx, rx_done_rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let mut recorded_8k = Vec::new();
                    while let Ok(frame) = pcm_rx.recv() {
                        recorded_8k.extend_from_slice(&frame);
                    }
                    let _ = rx_done_tx.send(recorded_8k);
                });

                // TX: play audio using paced writer.
                let tx_started_at_ms = started_at.elapsed().as_millis() as u64;
                let samples_8k = downsample_16k_to_8k(&request.tx_samples);

                let paced_tx = call.paced_pcm_writer().ok_or_else(|| {
                    LoadtestError::Protocol("xphone: paced_pcm_writer() returned None".into())
                })?;
                paced_tx.send(samples_8k).map_err(|_| {
                    LoadtestError::Protocol("xphone: paced_pcm_writer channel closed".into())
                })?;

                // Wait for paced playback to finish (approximately).
                let playback_duration =
                    Duration::from_millis((request.tx_samples.len() as u64 * 1000) / 16_000);
                std::thread::sleep(playback_duration);
                let tx_finished_at_ms = started_at.elapsed().as_millis() as u64;

                // Post-playback recording.
                if request.record_after_playback_ms > 0 {
                    std::thread::sleep(Duration::from_millis(request.record_after_playback_ms));
                }

                // Hang up.
                let _ = call.end();

                // Check if remote already hung up.
                let hangup_received = ended_rx.recv_timeout(Duration::from_secs(3)).is_ok();

                // Collect recorded audio.
                let recorded_8k = rx_done_rx
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap_or_default();
                let recorded_samples = upsample_8k_to_16k(&recorded_8k);

                Ok(Phase1CallResult {
                    connect_ms,
                    tx_started_at_ms,
                    tx_finished_at_ms,
                    recorded_samples,
                    hangup_received,
                })
            })
            .await??;

        Ok(result)
    }

    fn backend_name(&self) -> &'static str {
        "xphone"
    }

    async fn run_single_inbound_call(
        &self,
        request: Phase1InboundRequest,
    ) -> Result<Phase1CallResult, LoadtestError> {
        let incoming_rx = self.incoming_rx.clone();
        let call_timeout_ms = self.config.call_timeout_ms;

        let result =
            tokio::task::spawn_blocking(move || -> Result<Phase1CallResult, LoadtestError> {
                let started_at = Instant::now();

                // Wait for an inbound INVITE.
                let call = incoming_rx
                    .recv_timeout(Duration::from_millis(request.inbound_timeout_ms))
                    .map_err(|_| {
                        LoadtestError::Timeout(format!(
                            "no inbound call arrived within {}ms",
                            request.inbound_timeout_ms
                        ))
                    })?;

                // Accept (answer) the call.
                call.accept()
                    .map_err(|e| LoadtestError::Protocol(format!("xphone: accept failed: {e}")))?;

                // Wait for the call media to become active.
                wait_for_active(&call, Duration::from_millis(call_timeout_ms))?;

                debug!(
                    remote_ip = %call.remote_ip(),
                    remote_port = call.remote_port(),
                    media_active = call.media_session_active(),
                    remote_sdp = %call.remote_sdp().replace('\r', "").replace('\n', " | "),
                    "xphone: inbound media negotiated"
                );

                let connect_ms = started_at.elapsed().as_millis() as u64;

                // Set up ended notification.
                let (ended_tx, ended_rx) = crossbeam_channel::bounded::<()>(1);
                call.on_ended(move |reason| {
                    debug!(?reason, "xphone: inbound call ended");
                    let _ = ended_tx.try_send(());
                });

                // Settle delay.
                if request.settle_before_playback_ms > 0 {
                    std::thread::sleep(Duration::from_millis(request.settle_before_playback_ms));
                }

                // Start RX reader in a background thread.
                let pcm_rx = call.pcm_reader().ok_or_else(|| {
                    LoadtestError::Protocol("xphone: pcm_reader() returned None".into())
                })?;
                let (rx_done_tx, rx_done_rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let mut recorded_8k = Vec::new();
                    while let Ok(frame) = pcm_rx.recv() {
                        recorded_8k.extend_from_slice(&frame);
                    }
                    let _ = rx_done_tx.send(recorded_8k);
                });

                // TX: play audio using paced writer.
                let tx_started_at_ms = started_at.elapsed().as_millis() as u64;
                let samples_8k = downsample_16k_to_8k(&request.tx_samples);

                let paced_tx = call.paced_pcm_writer().ok_or_else(|| {
                    LoadtestError::Protocol("xphone: paced_pcm_writer() returned None".into())
                })?;
                paced_tx.send(samples_8k).map_err(|_| {
                    LoadtestError::Protocol("xphone: paced_pcm_writer channel closed".into())
                })?;

                // Wait for paced playback to finish (approximately).
                let playback_duration =
                    Duration::from_millis((request.tx_samples.len() as u64 * 1000) / 16_000);
                std::thread::sleep(playback_duration);
                let tx_finished_at_ms = started_at.elapsed().as_millis() as u64;

                // Post-playback recording.
                if request.record_after_playback_ms > 0 {
                    std::thread::sleep(Duration::from_millis(request.record_after_playback_ms));
                }

                // Hang up.
                let _ = call.end();

                // Check if remote already hung up.
                let hangup_received = ended_rx.recv_timeout(Duration::from_secs(3)).is_ok();

                // Collect recorded audio.
                let recorded_8k = rx_done_rx
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap_or_default();
                let recorded_samples = upsample_8k_to_16k(&recorded_8k);

                Ok(Phase1CallResult {
                    connect_ms,
                    tx_started_at_ms,
                    tx_finished_at_ms,
                    recorded_samples,
                    hangup_received,
                })
            })
            .await??;

        Ok(result)
    }
}

impl Drop for XphoneBackend {
    fn drop(&mut self) {
        debug!("xphone: disconnecting");
        let _ = self.phone.disconnect();
    }
}

/// Block the current thread until the call reaches `Active` state or timeout.
fn wait_for_active(call: &Arc<Call>, timeout: Duration) -> Result<(), LoadtestError> {
    match call.state() {
        CallState::Active | CallState::OnHold => return Ok(()),
        CallState::Ended => {
            return Err(LoadtestError::Protocol(
                "xphone: call ended before becoming active".into(),
            ));
        }
        _ => {}
    }

    let (active_tx, active_rx) = crossbeam_channel::bounded(1);
    let (failed_tx, failed_rx) = crossbeam_channel::bounded::<String>(1);

    call.on_state(move |state| match state {
        CallState::Active => {
            let _ = active_tx.try_send(());
        }
        CallState::Ended => {
            let _ = failed_tx.try_send("call ended before becoming active".into());
        }
        _ => {}
    });

    match call.state() {
        CallState::Active | CallState::OnHold => return Ok(()),
        CallState::Ended => {
            return Err(LoadtestError::Protocol(
                "xphone: call ended before becoming active".into(),
            ));
        }
        _ => {}
    }

    crossbeam_channel::select! {
        recv(active_rx) -> _ => Ok(()),
        recv(failed_rx) -> msg => {
            Err(LoadtestError::Protocol(format!(
                "xphone: call failed: {}",
                msg.unwrap_or_else(|_| "unknown".into())
            )))
        }
        default(timeout) => {
            Err(LoadtestError::Timeout(format!(
                "xphone: call did not become active within {}ms",
                timeout.as_millis()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsample_upsample_roundtrip() {
        let original_16k: Vec<i16> = (0..320).map(|i| (i * 100) as i16).collect();
        let down_8k = downsample_16k_to_8k(&original_16k);
        assert_eq!(down_8k.len(), 160);
        let up_16k = upsample_8k_to_16k(&down_8k);
        assert_eq!(up_16k.len(), 320);
        // Every even sample should match the original.
        for i in 0..160 {
            assert_eq!(up_16k[i * 2], original_16k[i * 2]);
        }
    }
}
