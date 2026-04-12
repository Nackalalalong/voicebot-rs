mod asterisk;

use async_trait::async_trait;

use crate::config::LoadtestConfig;
use crate::error::LoadtestError;

pub use asterisk::AsteriskExternalMediaBackend;

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

#[async_trait]
pub trait Phase1Backend: Send + Sync {
    async fn run_single_outbound_call(
        &self,
        request: Phase1CallRequest,
    ) -> Result<Phase1CallResult, LoadtestError>;

    fn backend_name(&self) -> &'static str;
}

pub fn build_backend(config: &LoadtestConfig) -> Result<Box<dyn Phase1Backend>, LoadtestError> {
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
        other => Err(LoadtestError::InvalidConfig(format!(
            "unsupported backend: {}",
            other
        ))),
    }
}
