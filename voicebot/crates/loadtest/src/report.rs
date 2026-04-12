use std::path::Path;

use serde::Serialize;

use crate::analysis::CallAnalysis;
use crate::error::LoadtestError;

#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    pub run_id: String,
    pub campaign_name: String,
    pub backend: String,
    pub target_endpoint: String,
    pub input_wav: String,
    pub artifact_dir: String,
    pub outcome: String,
    pub connect_ms: u64,
    pub tx_started_at_ms: u64,
    pub tx_finished_at_ms: u64,
    pub tx_duration_ms: u64,
    pub recorded_samples: usize,
    pub hangup_received: bool,
    pub tx_wav_path: String,
    pub rx_wav_path: String,
    pub analysis: CallAnalysis,
}

pub fn write_summary_json(path: &Path, summary: &RunSummary) -> Result<(), LoadtestError> {
    let json = serde_json::to_string_pretty(summary)?;
    std::fs::write(path, json)?;
    Ok(())
}
