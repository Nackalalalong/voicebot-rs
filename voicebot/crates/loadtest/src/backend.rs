mod asterisk;
mod xphone_backend;

use async_trait::async_trait;

use crate::config::LoadtestConfig;
use crate::error::LoadtestError;

pub use asterisk::AsteriskExternalMediaBackend;
pub use xphone_backend::XphoneBackend;

#[derive(Debug, Clone)]
pub struct Phase1CallRequest {
    pub target_endpoint: String,
    pub caller_id: String,
    pub tx_samples: Vec<i16>,
    pub settle_before_playback_ms: u64,
    pub record_after_playback_ms: u64,
}

#[derive(Debug, Clone)]
pub struct Phase1CallResult {
    pub connect_ms: u64,
    pub tx_started_at_ms: u64,
    pub tx_finished_at_ms: u64,
    pub recorded_samples: Vec<i16>,
    pub hangup_received: bool,
}

/// Request for an inbound call (phone waits for INVITE, then answers).
#[derive(Debug, Clone)]
pub struct Phase1InboundRequest {
    pub tx_samples: Vec<i16>,
    pub settle_before_playback_ms: u64,
    pub record_after_playback_ms: u64,
    pub inbound_timeout_ms: u64,
}

#[async_trait]
pub trait Phase1Backend: Send + Sync {
    async fn run_single_outbound_call(
        &self,
        request: Phase1CallRequest,
    ) -> Result<Phase1CallResult, LoadtestError>;

    /// Wait for an inbound call, answer, play TX audio, record RX, hang up.
    /// Default implementation returns an error (not all backends support inbound).
    async fn run_single_inbound_call(
        &self,
        _request: Phase1InboundRequest,
    ) -> Result<Phase1CallResult, LoadtestError> {
        Err(LoadtestError::InvalidConfig(format!(
            "backend '{}' does not support inbound mode",
            self.backend_name()
        )))
    }

    fn backend_name(&self) -> &'static str;
}

pub async fn build_backend(
    config: &LoadtestConfig,
) -> Result<Box<dyn Phase1Backend>, LoadtestError> {
    match config.backend.kind.as_str() {
        "asterisk-external-media" => {
            let cfg = config.backend.asterisk.clone().ok_or_else(|| {
                LoadtestError::InvalidConfig(
                    "backend.kind is 'asterisk-external-media' but [backend.asterisk] is missing"
                        .into(),
                )
            })?;
            Ok(Box::new(AsteriskExternalMediaBackend::new(cfg)))
        }
        "xphone" => {
            let cfg = config.backend.xphone.clone().ok_or_else(|| {
                LoadtestError::InvalidConfig(
                    "backend.kind is 'xphone' but [backend.xphone] is missing".into(),
                )
            })?;
            let backend = XphoneBackend::connect(cfg).await?;
            Ok(Box::new(backend))
        }
        other => Err(LoadtestError::InvalidConfig(format!(
            "unsupported backend: {}",
            other
        ))),
    }
}
