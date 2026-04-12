use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::backend::{Phase1Backend, Phase1CallRequest, Phase1CallResult};
use crate::config::AsteriskBackendConfig;
use crate::error::LoadtestError;

const RTP_HEADER_LEN: usize = 12;
const RTP_PAYLOAD_TYPE_PCMU: u8 = 0;
const RTP_FRAME_SAMPLES_ULAW: usize = 160;
const RTP_FRAME_MS: u64 = 20;
const MU_LAW_BIAS: i16 = 0x84;
const MU_LAW_CLIP: i16 = 32_635;
const MU_LAW_SEG_END: [i16; 8] = [
    0x00FF, 0x01FF, 0x03FF, 0x07FF, 0x0FFF, 0x1FFF, 0x3FFF, 0x7FFF,
];

pub struct AsteriskExternalMediaBackend {
    config: AsteriskBackendConfig,
    client: Client,
}

impl AsteriskExternalMediaBackend {
    pub fn new(config: AsteriskBackendConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }

    fn rest_client(&self) -> AriRestClient {
        AriRestClient {
            client: self.client.clone(),
            base_url: format!(
                "http://{}:{}/ari",
                self.config.ari_host, self.config.ari_port
            ),
            username: self.config.username.clone(),
            password: self.config.password.clone(),
        }
    }

    fn external_media_app_name(&self) -> String {
        format!("{}-loadtest-media", self.config.app_name)
    }
}

#[async_trait]
impl Phase1Backend for AsteriskExternalMediaBackend {
    async fn run_single_outbound_call(
        &self,
        request: Phase1CallRequest,
    ) -> Result<Phase1CallResult, LoadtestError> {
        let rest = self.rest_client();
        let started_at = Instant::now();
        let external_media_app = self.external_media_app_name();
        let mut event_monitor = AriAppMonitor::connect(
            &self.config,
            &[self.config.app_name.as_str(), external_media_app.as_str()],
        )
        .await?;

        let socket = Arc::new(UdpSocket::bind("0.0.0.0:0").await?);
        let listen_port = socket.local_addr()?.port();
        let external_host = format!("{}:{}", self.config.audio_host, listen_port);
        let stasis_timeout = Duration::from_millis(self.config.accept_timeout_ms);

        let external_media = rest
            .create_external_media(&external_media_app, &external_host, "ulaw")
            .await?;
        event_monitor
            .wait_for_stasis_start(&external_media.channel_id, stasis_timeout)
            .await?;
        let call_channel_id = rest
            .originate_in_app(
                &request.target_endpoint,
                &self.config.app_name,
                &request.caller_id,
            )
            .await?;
        event_monitor
            .wait_for_stasis_start(&call_channel_id, stasis_timeout)
            .await?;
        let bridge_id = rest
            .create_bridge(&format!("loadtest-{}", Uuid::new_v4()))
            .await?;
        rest.add_to_bridge(&bridge_id, &call_channel_id).await?;
        rest.add_to_bridge(&bridge_id, &external_media.channel_id)
            .await?;

        let (stop_tx, stop_rx) = watch::channel(false);
        let reader_handle = tokio::spawn(read_recorded_audio(
            Arc::clone(&socket),
            external_media.remote_addr,
            started_at,
            stop_rx,
        ));

        if request.settle_before_playback_ms > 0 {
            sleep(Duration::from_millis(request.settle_before_playback_ms)).await;
        }

        let tx_started_at_ms = started_at.elapsed().as_millis() as u64;
        play_audio(&socket, external_media.remote_addr, &request.tx_samples).await?;
        let tx_finished_at_ms = started_at.elapsed().as_millis() as u64;

        if request.record_after_playback_ms > 0 {
            sleep(Duration::from_millis(request.record_after_playback_ms)).await;
        }

        let _ = stop_tx.send(true);

        let _ = rest.hangup_channel(&call_channel_id).await;
        let _ = rest.hangup_channel(&external_media.channel_id).await;
        let _ = rest.destroy_bridge(&bridge_id).await;

        let reader_result = timeout(Duration::from_secs(3), reader_handle)
            .await
            .map_err(|_| {
                LoadtestError::Timeout("timed out waiting for RTP reader task".into())
            })???;
        let connect_ms = reader_result
            .first_packet_at_ms
            .unwrap_or(tx_started_at_ms);

        Ok(Phase1CallResult {
            connect_ms,
            tx_started_at_ms,
            tx_finished_at_ms,
            recorded_samples: reader_result.recorded_samples,
            hangup_received: reader_result.hangup_received,
        })
    }

    fn backend_name(&self) -> &'static str {
        "asterisk-external-media"
    }
}

struct AriRestClient {
    client: Client,
    base_url: String,
    username: String,
    password: String,
}

impl AriRestClient {
    async fn originate_in_app(
        &self,
        endpoint: &str,
        app_name: &str,
        caller_id: &str,
    ) -> Result<String, LoadtestError> {
        let url = format!("{}/channels", self.base_url);
        let response = self
            .client
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[
                ("endpoint", endpoint),
                ("app", app_name),
                ("callerId", caller_id),
            ])
            .send()
            .await?;
        let response = ensure_success(response, &url).await?;
        let body: serde_json::Value = response.json().await?;
        channel_id_from_body(&body)
    }

    async fn create_external_media(
        &self,
        app_name: &str,
        external_host: &str,
        format: &str,
    ) -> Result<ExternalMediaChannel, LoadtestError> {
        let url = format!("{}/channels/externalMedia", self.base_url);
        let response = self
            .client
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[
                ("app", app_name),
                ("external_host", external_host),
                ("format", format),
            ])
            .send()
            .await?;
        let response = ensure_success(response, &url).await?;
        let body: serde_json::Value = response.json().await?;
        external_media_from_body(&body)
    }

    async fn create_bridge(&self, name: &str) -> Result<String, LoadtestError> {
        let url = format!("{}/bridges", self.base_url);
        let response = self
            .client
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[("type", "mixing"), ("name", name)])
            .send()
            .await?;
        let response = ensure_success(response, &url).await?;
        let body: serde_json::Value = response.json().await?;
        channel_id_from_body(&body)
    }

    async fn add_to_bridge(&self, bridge_id: &str, channel_id: &str) -> Result<(), LoadtestError> {
        let url = format!("{}/bridges/{}/addChannel", self.base_url, bridge_id);
        let response = self
            .client
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[("channel", channel_id)])
            .send()
            .await?;
        let _ = ensure_success(response, &url).await?;
        Ok(())
    }

    async fn hangup_channel(&self, channel_id: &str) -> Result<(), LoadtestError> {
        let url = format!("{}/channels/{}", self.base_url, channel_id);
        let response = self
            .client
            .delete(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[("reason", "normal")])
            .send()
            .await?;
        if response.status().is_success() || response.status().as_u16() == 404 {
            return Ok(());
        }
        let _ = ensure_success(response, &url).await?;
        Ok(())
    }

    async fn destroy_bridge(&self, bridge_id: &str) -> Result<(), LoadtestError> {
        let url = format!("{}/bridges/{}", self.base_url, bridge_id);
        let response = self
            .client
            .delete(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await?;
        if response.status().is_success() || response.status().as_u16() == 404 {
            return Ok(());
        }
        let _ = ensure_success(response, &url).await?;
        Ok(())
    }
}

struct ExternalMediaChannel {
    channel_id: String,
    remote_addr: SocketAddr,
}

struct AriAppMonitor {
    stasis_starts_rx: mpsc::Receiver<String>,
    seen_channel_ids: HashSet<String>,
    reader_task: tokio::task::JoinHandle<()>,
}

impl AriAppMonitor {
    async fn connect(
        config: &AsteriskBackendConfig,
        app_names: &[&str],
    ) -> Result<Self, LoadtestError> {
        let ws_url = format!(
            "ws://{}:{}/ari/events?api_key={}:{}&app={}",
            config.ari_host,
            config.ari_port,
            config.username,
            config.password,
            app_names.join(",")
        );
        let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .map_err(|error| {
                LoadtestError::Protocol(format!(
                    "failed to connect ARI websocket {}: {}",
                    ws_url, error
                ))
            })?;
        let (_, mut ws_rx) = ws_stream.split();
        let (stasis_starts_tx, stasis_starts_rx) = mpsc::channel(32);
        let reader_task = tokio::spawn(async move {
            while let Some(message_result) = ws_rx.next().await {
                let text = match message_result {
                    Ok(Message::Text(text)) => text,
                    Ok(Message::Close(_)) => break,
                    Ok(_) => continue,
                    Err(_) => break,
                };

                let Ok(event) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                if event.get("type").and_then(serde_json::Value::as_str) != Some("StasisStart") {
                    continue;
                }
                let Some(channel_id) = event
                    .get("channel")
                    .and_then(|channel| channel.get("id"))
                    .and_then(serde_json::Value::as_str)
                else {
                    continue;
                };
                if stasis_starts_tx.send(channel_id.to_string()).await.is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            stasis_starts_rx,
            seen_channel_ids: HashSet::new(),
            reader_task,
        })
    }

    async fn wait_for_stasis_start(
        &mut self,
        channel_id: &str,
        wait_timeout: Duration,
    ) -> Result<(), LoadtestError> {
        if self.seen_channel_ids.contains(channel_id) {
            return Ok(());
        }

        let channel_id = channel_id.to_string();
        timeout(wait_timeout, async {
            loop {
                let Some(seen_channel_id) = self.stasis_starts_rx.recv().await else {
                    return Err(LoadtestError::Protocol(
                        "ARI websocket closed while waiting for StasisStart".into(),
                    ));
                };
                self.seen_channel_ids.insert(seen_channel_id.clone());
                if seen_channel_id == channel_id {
                    return Ok(());
                }
            }
        })
        .await
        .map_err(|_| {
            LoadtestError::Timeout(format!(
                "timed out waiting for StasisStart on channel {}",
                channel_id
            ))
        })?
    }
}

impl Drop for AriAppMonitor {
    fn drop(&mut self) {
        self.reader_task.abort();
    }
}

async fn ensure_success(
    response: reqwest::Response,
    url: &str,
) -> Result<reqwest::Response, LoadtestError> {
    let status = response.status().as_u16();
    if (200..300).contains(&status) {
        Ok(response)
    } else {
        let body = response.text().await.unwrap_or_default();
        Err(LoadtestError::Protocol(format!(
            "ARI REST error {} for {}: {}",
            status, url, body
        )))
    }
}

fn channel_id_from_body(body: &serde_json::Value) -> Result<String, LoadtestError> {
    body.get("id")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| LoadtestError::Protocol("ARI response missing 'id'".into()))
}

fn external_media_from_body(body: &serde_json::Value) -> Result<ExternalMediaChannel, LoadtestError> {
    let channel_id = channel_id_from_body(body)?;
    let channelvars = body
        .get("channelvars")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| LoadtestError::Protocol("ARI externalMedia response missing channelvars".into()))?;
    let local_address = channelvars
        .get("UNICASTRTP_LOCAL_ADDRESS")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| LoadtestError::Protocol("ARI externalMedia response missing UNICASTRTP_LOCAL_ADDRESS".into()))?;
    let local_port = channelvars
        .get("UNICASTRTP_LOCAL_PORT")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| LoadtestError::Protocol("ARI externalMedia response missing UNICASTRTP_LOCAL_PORT".into()))?;
    let remote_addr = format!("{}:{}", local_address, local_port)
        .parse()
        .map_err(|error| {
            LoadtestError::Protocol(format!(
                "failed to parse external media RTP address {}:{}: {}",
                local_address, local_port, error
            ))
        })?;

    Ok(ExternalMediaChannel {
        channel_id,
        remote_addr,
    })
}

struct ReaderResult {
    recorded_samples: Vec<i16>,
    first_packet_at_ms: Option<u64>,
    hangup_received: bool,
}

async fn read_recorded_audio(
    socket: Arc<UdpSocket>,
    remote_addr: SocketAddr,
    started_at: Instant,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<ReaderResult, LoadtestError> {
    let mut packet_buffer = [0u8; 2048];
    let mut recorded_8k_samples = Vec::new();
    let mut first_packet_at_ms = None;
    let mut learned_source_addr = None;

    loop {
        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() {
                    break;
                }
            }
            recv_result = socket.recv_from(&mut packet_buffer) => {
                let (received, source_addr) = recv_result?;
                if learned_source_addr.is_none() {
                    learned_source_addr = Some(source_addr);
                }
                if learned_source_addr != Some(source_addr) && source_addr != remote_addr {
                    continue;
                }
                let Some(payload) = parse_rtp_payload(&packet_buffer[..received]) else {
                    continue;
                };
                if payload.is_empty() {
                    continue;
                }
                if first_packet_at_ms.is_none() {
                    first_packet_at_ms = Some(started_at.elapsed().as_millis() as u64);
                }
                recorded_8k_samples.extend(payload.iter().copied().map(mulaw_to_linear));
            }
        }
    }

    Ok(ReaderResult {
        recorded_samples: upsample_8k_to_16k(&recorded_8k_samples),
        first_packet_at_ms,
        hangup_received: false,
    })
}

async fn play_audio(
    socket: &UdpSocket,
    remote_addr: SocketAddr,
    samples: &[i16],
) -> Result<(), LoadtestError> {
    let ulaw_samples = downsample_16k_to_8k(samples);
    let seed = Uuid::new_v4();
    let seed_bytes = seed.as_bytes();
    let mut sequence = u16::from_be_bytes([seed_bytes[0], seed_bytes[1]]);
    let mut timestamp = u32::from_be_bytes([seed_bytes[2], seed_bytes[3], seed_bytes[4], seed_bytes[5]]);
    let ssrc = u32::from_be_bytes([seed_bytes[6], seed_bytes[7], seed_bytes[8], seed_bytes[9]]);

    for chunk in ulaw_samples.chunks(RTP_FRAME_SAMPLES_ULAW) {
        let mut padded_chunk = chunk.to_vec();
        if padded_chunk.len() < RTP_FRAME_SAMPLES_ULAW {
            padded_chunk.resize(RTP_FRAME_SAMPLES_ULAW, 0);
        }
        let packet = build_rtp_packet(&padded_chunk, sequence, timestamp, ssrc);
        socket.send_to(&packet, remote_addr).await?;
        sequence = sequence.wrapping_add(1);
        timestamp = timestamp.wrapping_add(RTP_FRAME_SAMPLES_ULAW as u32);
        sleep(Duration::from_millis(RTP_FRAME_MS)).await;
    }

    Ok(())
}

fn build_rtp_packet(samples: &[i16], sequence: u16, timestamp: u32, ssrc: u32) -> Vec<u8> {
    let mut packet = Vec::with_capacity(RTP_HEADER_LEN + samples.len());
    packet.push(0x80);
    packet.push(RTP_PAYLOAD_TYPE_PCMU);
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.extend_from_slice(&timestamp.to_be_bytes());
    packet.extend_from_slice(&ssrc.to_be_bytes());
    packet.extend(samples.iter().copied().map(linear_to_mulaw));
    packet
}

fn parse_rtp_payload(packet: &[u8]) -> Option<&[u8]> {
    if packet.len() < RTP_HEADER_LEN {
        return None;
    }
    if packet[0] >> 6 != 2 {
        return None;
    }

    let csrc_count = (packet[0] & 0x0F) as usize;
    let has_extension = packet[0] & 0x10 != 0;
    let has_padding = packet[0] & 0x20 != 0;

    let mut offset = RTP_HEADER_LEN + (csrc_count * 4);
    if packet.len() < offset {
        return None;
    }

    if has_extension {
        if packet.len() < offset + 4 {
            return None;
        }
        let extension_length_words = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]) as usize;
        offset += 4 + (extension_length_words * 4);
        if packet.len() < offset {
            return None;
        }
    }

    let payload_end = if has_padding {
        let padding = *packet.last()? as usize;
        packet.len().checked_sub(padding)?
    } else {
        packet.len()
    };

    if payload_end < offset {
        return None;
    }

    Some(&packet[offset..payload_end])
}

fn downsample_16k_to_8k(samples: &[i16]) -> Vec<i16> {
    samples.iter().step_by(2).copied().collect()
}

fn upsample_8k_to_16k(samples: &[i16]) -> Vec<i16> {
    let mut upsampled = Vec::with_capacity(samples.len() * 2);
    for &sample in samples {
        upsampled.push(sample);
        upsampled.push(sample);
    }
    upsampled
}

fn linear_to_mulaw(sample: i16) -> u8 {
    let mut magnitude = sample;
    let sign_mask = if magnitude < 0 {
        magnitude = MU_LAW_BIAS.saturating_sub(magnitude);
        0x7F
    } else {
        magnitude = magnitude.saturating_add(MU_LAW_BIAS);
        0xFF
    };
    magnitude = magnitude.min(MU_LAW_CLIP);

    let segment = MU_LAW_SEG_END
        .iter()
        .position(|&segment_end| magnitude <= segment_end)
        .unwrap_or(MU_LAW_SEG_END.len());
    if segment >= MU_LAW_SEG_END.len() {
        return 0x7F ^ sign_mask;
    }

    let mantissa = ((magnitude >> (segment + 3)) & 0x0F) as u8;
    (((segment as u8) << 4) | mantissa) ^ sign_mask
}

fn mulaw_to_linear(encoded: u8) -> i16 {
    let inverted = !encoded;
    let mut magnitude = (((inverted & 0x0F) as i16) << 3) + MU_LAW_BIAS;
    magnitude <<= ((inverted & 0x70) >> 4) as usize;

    if inverted & 0x80 != 0 {
        MU_LAW_BIAS.saturating_sub(magnitude)
    } else {
        magnitude.saturating_sub(MU_LAW_BIAS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mulaw_roundtrip_preserves_basic_shape() {
        let source = [-20_000, -4_000, -100, 0, 100, 4_000, 20_000];
        let decoded: Vec<i16> = source
            .iter()
            .copied()
            .map(linear_to_mulaw)
            .map(mulaw_to_linear)
            .collect();

        assert!(decoded[0] < decoded[1]);
        assert!(decoded[1] < decoded[2]);
        assert!(decoded[2] <= decoded[3]);
        assert!(decoded[3] <= decoded[4]);
        assert!(decoded[4] < decoded[5]);
        assert!(decoded[5] < decoded[6]);
        assert!(decoded[3].abs() < 1_000);
    }

    #[test]
    fn parses_minimal_rtp_payload() {
        let packet = build_rtp_packet(&[0, 1000, -1000], 1, 160, 42);
        let payload = parse_rtp_payload(&packet).expect("payload");

        assert_eq!(payload.len(), 3);
    }
}
