use std::path::PathBuf;

use uuid::Uuid;

use crate::analysis::analyze_received_audio;
use crate::audio::{load_and_normalize_wav, write_wav};
use crate::backend::{build_backend, Phase1CallRequest};
use crate::config::LoadtestConfig;
use crate::error::LoadtestError;
use crate::report::{write_summary_json, RunSummary};

pub async fn run_phase1(config: &LoadtestConfig) -> Result<RunSummary, LoadtestError> {
    let normalized_audio = load_and_normalize_wav(&config.media.input_wav)?;

    let run_id = Uuid::new_v4().to_string();
    let run_dir = config.media.artifact_dir.join(&run_id);
    std::fs::create_dir_all(&run_dir)?;

    std::fs::write(
        run_dir.join("config.resolved.toml"),
        config.to_toml_string()?,
    )?;

    let tx_wav_path = run_dir.join("tx.normalized.wav");
    let rx_wav_path = run_dir.join("rx.received.wav");
    let summary_path = run_dir.join("summary.json");

    write_wav(&tx_wav_path, &normalized_audio.samples)?;

    let backend = build_backend(config)?;
    let backend_result = backend
        .run_single_outbound_call(Phase1CallRequest {
            target_endpoint: config.campaign.target_endpoint.clone(),
            caller_id: config.campaign.caller_id.clone(),
            tx_samples: normalized_audio.samples.clone(),
            settle_before_playback_ms: config.campaign.settle_before_playback_ms,
            record_after_playback_ms: config.campaign.record_after_playback_ms,
        })
        .await?;

    write_wav(&rx_wav_path, &backend_result.recorded_samples)?;

    let analysis = analyze_received_audio(
        &backend_result.recorded_samples,
        backend_result.tx_finished_at_ms,
        &config.analysis,
    );
    let outcome = match analysis.first_response_ms {
        Some(_) => "success",
        None if backend_result.recorded_samples.is_empty() => "no_received_audio",
        None => "completed_without_detected_response",
    };

    let summary = RunSummary {
        run_id,
        campaign_name: config.campaign.name.clone(),
        backend: backend.backend_name().to_string(),
        target_endpoint: config.campaign.target_endpoint.clone(),
        input_wav: path_string(&config.media.input_wav),
        artifact_dir: path_string(&run_dir),
        outcome: outcome.into(),
        connect_ms: backend_result.connect_ms,
        tx_started_at_ms: backend_result.tx_started_at_ms,
        tx_finished_at_ms: backend_result.tx_finished_at_ms,
        tx_duration_ms: normalized_audio.duration_ms(),
        recorded_samples: backend_result.recorded_samples.len(),
        hangup_received: backend_result.hangup_received,
        tx_wav_path: path_string(&tx_wav_path),
        rx_wav_path: path_string(&rx_wav_path),
        analysis,
    };

    write_summary_json(&summary_path, &summary)?;
    Ok(summary)
}

fn path_string(path: &PathBuf) -> String {
    path.to_string_lossy().into_owned()
}
