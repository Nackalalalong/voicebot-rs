use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::Client;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use crate::audio::{samples_to_pcm_bytes, TEN_MS_SAMPLES};
use crate::backend::{Phase1Backend, Phase1CallRequest, Phase1CallResult};
use crate::config::AsteriskBackendConfig;
use crate::error::LoadtestError;

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
}

#[async_trait]
impl Phase1Backend for AsteriskExternalMediaBackend {
    async fn run_single_outbound_call(
        &self,
        request: Phase1CallRequest,
    ) -> Result<Phase1CallResult, LoadtestError> {
        let rest = self.rest_client();
        let started_at = Instant::now();

        let listener = TcpListener::bind("0.0.0.0:0").await?;
        let listen_port = listener.local_addr()?.port();
        let external_host = format!("{}:{}", self.config.audio_host, listen_port);

        let ext_channel_id = rest
            .create_external_media(&self.config.app_name, &external_host, "slin16")
            .await?;
        let bridge_id = rest
            .create_bridge(&format!("loadtest-{}", Uuid::new_v4()))
            .await?;
        let call_channel_id = rest
            .originate_in_app(
                &request.target_endpoint,
                &self.config.app_name,
                &request.caller_id,
            )
            .await?;
        rest.add_to_bridge(&bridge_id, &[&call_channel_id, &ext_channel_id])
            .await?;

        let accept_timeout = Duration::from_millis(self.config.accept_timeout_ms);
        let (tcp_stream, _) =
            timeout(accept_timeout, listener.accept())
                .await
                .map_err(|_| {
                    LoadtestError::Timeout(
                        "timed out waiting for AudioSocket TCP connection".into(),
                    )
                })??;
        drop(listener);

        let connect_ms = started_at.elapsed().as_millis() as u64;
        let (mut read_half, mut write_half) = tokio::io::split(tcp_stream);

        let reader_handle = tokio::spawn(async move { read_recorded_audio(&mut read_half).await });

        if request.settle_before_playback_ms > 0 {
            sleep(Duration::from_millis(request.settle_before_playback_ms)).await;
        }

        let tx_started_at_ms = started_at.elapsed().as_millis() as u64;
        play_audio(&mut write_half, &request.tx_samples).await?;
        let tx_finished_at_ms = started_at.elapsed().as_millis() as u64;

        if request.record_after_playback_ms > 0 {
            sleep(Duration::from_millis(request.record_after_playback_ms)).await;
        }

        let _ = write_hangup_packet(&mut write_half).await;
        let _ = write_half.shutdown().await;

        let _ = rest.hangup_channel(&call_channel_id).await;
        let _ = rest.hangup_channel(&ext_channel_id).await;
        let _ = rest.destroy_bridge(&bridge_id).await;

        let reader_result = timeout(Duration::from_secs(3), reader_handle)
            .await
            .map_err(|_| {
                LoadtestError::Timeout("timed out waiting for AudioSocket reader task".into())
            })???;

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
        ensure_success(response.status().as_u16(), &url)?;
        let body: serde_json::Value = response.json().await?;
        channel_id_from_body(&body)
    }

    async fn create_external_media(
        &self,
        app_name: &str,
        external_host: &str,
        format: &str,
    ) -> Result<String, LoadtestError> {
        let url = format!("{}/channels/externalMedia", self.base_url);
        let response = self
            .client
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[
                ("app", app_name),
                ("external_host", external_host),
                ("transport", "tcp"),
                ("encapsulation", "audiosocket"),
                ("format", format),
                ("direction", "both"),
            ])
            .send()
            .await?;
        ensure_success(response.status().as_u16(), &url)?;
        let body: serde_json::Value = response.json().await?;
        channel_id_from_body(&body)
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
        ensure_success(response.status().as_u16(), &url)?;
        let body: serde_json::Value = response.json().await?;
        channel_id_from_body(&body)
    }

    async fn add_to_bridge(
        &self,
        bridge_id: &str,
        channel_ids: &[&str],
    ) -> Result<(), LoadtestError> {
        let url = format!("{}/bridges/{}/addChannel", self.base_url, bridge_id);
        let joined = channel_ids.join(",");
        let response = self
            .client
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[("channel", joined.as_str())])
            .send()
            .await?;
        ensure_success(response.status().as_u16(), &url)?;
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
        ensure_success(response.status().as_u16(), &url)
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
        ensure_success(response.status().as_u16(), &url)
    }
}

fn ensure_success(status: u16, url: &str) -> Result<(), LoadtestError> {
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(LoadtestError::Protocol(format!(
            "ARI REST error {} for {}",
            status, url
        )))
    }
}

fn channel_id_from_body(body: &serde_json::Value) -> Result<String, LoadtestError> {
    body.get("id")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| LoadtestError::Protocol("ARI response missing 'id'".into()))
}

struct ReaderResult {
    recorded_samples: Vec<i16>,
    hangup_received: bool,
}

async fn read_recorded_audio<R>(reader: &mut R) -> Result<ReaderResult, LoadtestError>
where
    R: AsyncReadExt + Unpin,
{
    let mut recorded_samples = Vec::new();
    let mut hangup_received = false;

    loop {
        match read_packet(reader).await {
            Ok(packet) => match packet.kind {
                0x00 => {
                    hangup_received = true;
                    break;
                }
                0x01 => {}
                0x10 => {
                    recorded_samples.extend(
                        packet
                            .payload
                            .chunks_exact(2)
                            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]])),
                    );
                }
                _ => {}
            },
            Err(LoadtestError::Io(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(LoadtestError::Io(error))
                if error.kind() == std::io::ErrorKind::ConnectionReset =>
            {
                break;
            }
            Err(error) => return Err(error),
        }
    }

    Ok(ReaderResult {
        recorded_samples,
        hangup_received,
    })
}

async fn play_audio<W>(writer: &mut W, samples: &[i16]) -> Result<(), LoadtestError>
where
    W: AsyncWriteExt + Unpin,
{
    let pcm_bytes = samples_to_pcm_bytes(samples);
    let bytes_per_chunk = TEN_MS_SAMPLES * 2;
    for chunk in pcm_bytes.chunks(bytes_per_chunk) {
        let mut owned_chunk = chunk.to_vec();
        if owned_chunk.len() < bytes_per_chunk {
            owned_chunk.resize(bytes_per_chunk, 0);
        }
        write_audio_packet(writer, &owned_chunk).await?;
        sleep(Duration::from_millis(10)).await;
    }
    Ok(())
}

struct AudioSocketPacket {
    kind: u8,
    payload: Vec<u8>,
}

async fn read_packet<R>(reader: &mut R) -> Result<AudioSocketPacket, LoadtestError>
where
    R: AsyncReadExt + Unpin,
{
    let kind = reader.read_u8().await?;
    let length = reader.read_u16().await?;
    let mut payload = vec![0u8; length as usize];
    if length > 0 {
        reader.read_exact(&mut payload).await?;
    }
    Ok(AudioSocketPacket { kind, payload })
}

async fn write_audio_packet<W>(writer: &mut W, pcm_bytes: &[u8]) -> Result<(), LoadtestError>
where
    W: AsyncWriteExt + Unpin,
{
    writer.write_u8(0x10).await?;
    writer.write_u16(pcm_bytes.len() as u16).await?;
    writer.write_all(pcm_bytes).await?;
    Ok(())
}

async fn write_hangup_packet<W>(writer: &mut W) -> Result<(), LoadtestError>
where
    W: AsyncWriteExt + Unpin,
{
    writer.write_u8(0x00).await?;
    writer.write_u16(0).await?;
    Ok(())
}
