use std::net::SocketAddr;

use common::config::AsteriskConfig;
use reqwest::Client;

use crate::error::AriError;

#[derive(Clone, Debug)]
pub struct ExternalMediaChannel {
    pub id: String,
    pub remote_addr: SocketAddr,
}

/// Thin async wrapper around the Asterisk REST Interface.
///
/// Uses HTTP Basic auth for every request. The base URL comes from
/// `AsteriskConfig` (`http://{ari_host}:{ari_port}`).
#[derive(Clone)]
pub struct AriRestClient {
    client: Client,
    base_url: String,
    username: String,
    password: String,
}

impl AriRestClient {
    pub fn new(config: &AsteriskConfig) -> Self {
        Self {
            client: Client::new(),
            base_url: format!("http://{}:{}/ari", config.ari_host, config.ari_port),
            username: config.username.clone(),
            password: config.password.clone(),
        }
    }

    /// GET /asterisk/info
    pub async fn asterisk_info(&self) -> Result<serde_json::Value, AriError> {
        let url = format!("{}/asterisk/info", self.base_url);
        let resp = self
            .client
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(AriError::Rest {
                status: resp.status().as_u16(),
                url,
            });
        }
        Ok(resp.json().await?)
    }

    /// GET /endpoints/{technology}/{resource}
    pub async fn endpoint(
        &self,
        technology: &str,
        resource: &str,
    ) -> Result<serde_json::Value, AriError> {
        let url = format!("{}/endpoints/{}/{}", self.base_url, technology, resource);
        let resp = self
            .client
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(AriError::Rest {
                status: resp.status().as_u16(),
                url,
            });
        }
        Ok(resp.json().await?)
    }

    /// POST /channels?endpoint=...&app=... — create a channel directly into a Stasis app.
    ///
    /// Returns the new channel ID.
    pub async fn originate_in_app(
        &self,
        endpoint: &str,
        app_name: &str,
        caller_id: &str,
    ) -> Result<String, AriError> {
        let url = format!("{}/channels", self.base_url);
        let resp = self
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
        if !resp.status().is_success() {
            return Err(AriError::Rest {
                status: resp.status().as_u16(),
                url,
            });
        }
        let body: serde_json::Value = resp.json().await?;
        body["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| AriError::Protocol("channel response missing 'id'".into()))
    }

    /// POST /channels/{channelId}/answer
    pub async fn answer_channel(&self, channel_id: &str) -> Result<(), AriError> {
        let url = format!("{}/channels/{}/answer", self.base_url, channel_id);
        let resp = self
            .client
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(AriError::Rest {
                status: resp.status().as_u16(),
                url,
            });
        }
        Ok(())
    }

    /// POST /channels/externalMedia — create an RTP external-media channel.
    ///
    /// Returns the new external-media channel plus the address Asterisk expects RTP to be sent to.
    pub async fn create_external_media(
        &self,
        app_name: &str,
        external_host: &str,
        format: &str,
    ) -> Result<ExternalMediaChannel, AriError> {
        let url = format!("{}/channels/externalMedia", self.base_url);
        let resp = self
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
        if !resp.status().is_success() {
            return Err(AriError::Rest {
                status: resp.status().as_u16(),
                url,
            });
        }
        let body: serde_json::Value = resp.json().await?;
        let id = body["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| AriError::Protocol("externalMedia response missing 'id'".into()))?;
        let channelvars = body["channelvars"]
            .as_object()
            .ok_or_else(|| AriError::Protocol("externalMedia response missing 'channelvars'".into()))?;
        let local_address = channelvars
            .get("UNICASTRTP_LOCAL_ADDRESS")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AriError::Protocol(
                    "externalMedia response missing UNICASTRTP_LOCAL_ADDRESS".into(),
                )
            })?;
        let local_port = channelvars
            .get("UNICASTRTP_LOCAL_PORT")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AriError::Protocol(
                    "externalMedia response missing UNICASTRTP_LOCAL_PORT".into(),
                )
            })?;
        let remote_addr = format!("{}:{}", local_address, local_port)
            .parse()
            .map_err(|error| {
                AriError::Protocol(format!(
                    "invalid externalMedia RTP address {}:{}: {}",
                    local_address, local_port, error
                ))
            })?;

        Ok(ExternalMediaChannel { id, remote_addr })
    }

    /// POST /bridges?type=mixing — create a mixing bridge.
    ///
    /// Returns the bridge ID.
    pub async fn create_bridge(&self, name: &str) -> Result<String, AriError> {
        let url = format!("{}/bridges", self.base_url);
        let resp = self
            .client
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[("type", "mixing"), ("name", name)])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(AriError::Rest {
                status: resp.status().as_u16(),
                url,
            });
        }
        let body: serde_json::Value = resp.json().await?;
        body["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| AriError::Protocol("bridge response missing 'id'".into()))
    }

    /// POST /bridges/{bridgeId}/addChannel?channel=id1,id2
    pub async fn add_to_bridge(
        &self,
        bridge_id: &str,
        channel_ids: &[&str],
    ) -> Result<(), AriError> {
        let url = format!("{}/bridges/{}/addChannel", self.base_url, bridge_id);
        let channels = channel_ids.join(",");
        let resp = self
            .client
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[("channel", channels.as_str())])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(AriError::Rest {
                status: resp.status().as_u16(),
                url,
            });
        }
        Ok(())
    }

    /// DELETE /channels/{channelId}?reason=normal — hang up a channel.
    pub async fn hangup_channel(&self, channel_id: &str) -> Result<(), AriError> {
        let url = format!("{}/channels/{}", self.base_url, channel_id);
        self.client
            .delete(&url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[("reason", "normal")])
            .send()
            .await?;
        Ok(())
    }

    /// DELETE /bridges/{bridgeId} — destroy a bridge.
    pub async fn destroy_bridge(&self, bridge_id: &str) -> Result<(), AriError> {
        let url = format!("{}/bridges/{}", self.base_url, bridge_id);
        self.client
            .delete(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await?;
        Ok(())
    }
}
