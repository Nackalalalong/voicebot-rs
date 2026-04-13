use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use uuid::Uuid;

use crate::analysis::analyze_received_audio;
use crate::audio::{load_and_normalize_wav, write_wav};
use crate::backend::{build_backend, Phase1Backend, Phase1CallRequest, Phase1CallResult};
use crate::config::LoadtestConfig;
use crate::error::LoadtestError;
use crate::report::{
    write_campaign_report_md, write_campaign_summary_json, write_summary_json, CallResult,
    CampaignSummary, RunSummary,
};

// ── Campaign runner ───────────────────────────────────────────────────────────

/// Run a full multi-call campaign with concurrency, rate limiting, and ramp-up.
/// Writes per-call rx WAVs, a campaign JSON summary, and a Markdown report to
/// `{artifact_dir}/{campaign_id}/`.
pub async fn run_campaign(config: &LoadtestConfig) -> Result<CampaignSummary, LoadtestError> {
    let concurrency = config.campaign.concurrency;
    let total_calls = config.campaign.total_calls; // 0 = unlimited (soak mode)
    let soak_deadline: Option<Instant> = if config.campaign.soak_duration_secs > 0 {
        Some(Instant::now() + Duration::from_secs(config.campaign.soak_duration_secs))
    } else {
        None
    };

    let normalized_audio = load_and_normalize_wav(&config.media.input_wav)?;

    let campaign_id = Uuid::new_v4().to_string();
    let campaign_dir = config.media.artifact_dir.join(&campaign_id);
    let calls_dir = campaign_dir.join("calls");
    std::fs::create_dir_all(&calls_dir)?;

    // Write the normalized TX wav once at campaign level.
    let tx_wav_path = campaign_dir.join("tx.normalized.wav");
    write_wav(&tx_wav_path, &normalized_audio.samples)?;

    std::fs::write(
        campaign_dir.join("config.resolved.toml"),
        config.to_toml_string()?,
    )?;

    let backend: Arc<dyn Phase1Backend> = Arc::from(build_backend(config)?);
    let backend_name = backend.backend_name().to_string();
    let started_at = Instant::now();

    // --- Concurrent call loop ------------------------------------------------
    // We maintain a FuturesUnordered pool bounded to `concurrency`.  The main
    // loop alternates between spawning new tasks (with rate-limiting/ramp-up
    // delays) and draining completed results.

    let mut pending: FuturesUnordered<
        JoinHandle<(usize, Result<Phase1CallResult, LoadtestError>)>,
    > = FuturesUnordered::new();
    let mut call_index = 0usize;
    let mut raw_results: Vec<(usize, Result<Phase1CallResult, LoadtestError>)> = Vec::new();

    loop {
        let within_deadline = soak_deadline.map_or(true, |d| Instant::now() < d);
        let within_count = total_calls == 0 || call_index < total_calls;
        let can_spawn = within_deadline && within_count;

        if can_spawn && pending.len() < concurrency {
            // Ramp-up: stagger the first `concurrency` calls evenly over ramp_up_ms.
            if config.campaign.ramp_up_ms > 0 && call_index < concurrency {
                let delay = Duration::from_millis(
                    (call_index as u64 * config.campaign.ramp_up_ms) / concurrency as u64,
                );
                let elapsed = started_at.elapsed();
                if elapsed < delay {
                    sleep(delay - elapsed).await;
                }
            }

            // Rate limiting: call N should not start before N / rate seconds elapsed.
            if config.campaign.call_rate_per_second > 0.0 {
                let expected = Duration::from_secs_f64(
                    call_index as f64 / config.campaign.call_rate_per_second,
                );
                let elapsed = started_at.elapsed();
                if elapsed < expected {
                    tokio::select! {
                        _ = sleep(expected - elapsed) => {},
                        // While waiting for rate-limit slot, keep draining finished tasks.
                        result = pending.next(), if !pending.is_empty() => {
                            if let Some(r) = result {
                                raw_results.push(r?);
                            }
                            continue;
                        }
                    }
                }
            }

            let idx = call_index;
            call_index += 1;
            let backend = Arc::clone(&backend);
            let request = Phase1CallRequest {
                target_endpoint: config.campaign.target_endpoint.clone(),
                caller_id: config.campaign.caller_id.clone(),
                tx_samples: normalized_audio.samples.clone(),
                settle_before_playback_ms: config.campaign.settle_before_playback_ms,
                record_after_playback_ms: config.campaign.record_after_playback_ms,
            };
            pending.push(tokio::spawn(async move {
                (idx, backend.run_single_outbound_call(request).await)
            }));
        } else if pending.is_empty() {
            // All calls spawned and all results collected.
            break;
        } else {
            // Pool is full or no more calls to spawn — wait for the next result.
            if let Some(result) = pending.next().await {
                raw_results.push(result?);
            }
        }
    }

    let duration_ms = started_at.elapsed().as_millis() as u64;

    // --- Build per-call results and write rx WAVs ----------------------------
    raw_results.sort_by_key(|(idx, _)| *idx);
    let mut call_results: Vec<CallResult> = Vec::with_capacity(raw_results.len());

    for (idx, result) in raw_results {
        let call_dir = calls_dir.join(format!("{:04}", idx));
        std::fs::create_dir_all(&call_dir)?;

        let rx_wav_path = call_dir.join("rx.wav");

        match result {
            Ok(backend_result) => {
                write_wav(&rx_wav_path, &backend_result.recorded_samples)?;
                let analysis = analyze_received_audio(
                    &backend_result.recorded_samples,
                    backend_result.tx_finished_at_ms,
                    &config.analysis,
                );
                let outcome = call_outcome(&backend_result.recorded_samples, &analysis);
                call_results.push(CallResult {
                    call_index: idx,
                    outcome,
                    error: None,
                    connect_ms: backend_result.connect_ms,
                    tx_started_at_ms: backend_result.tx_started_at_ms,
                    tx_finished_at_ms: backend_result.tx_finished_at_ms,
                    recorded_samples: backend_result.recorded_samples.len(),
                    hangup_received: backend_result.hangup_received,
                    analysis: Some(analysis),
                    rx_wav_path: rx_wav_path.to_string_lossy().into_owned(),
                });
            }
            Err(error) => {
                // Write an empty WAV so the artifact directory stays consistent.
                let _ = write_wav(&rx_wav_path, &[]);
                call_results.push(CallResult {
                    call_index: idx,
                    outcome: "failed".into(),
                    error: Some(error.to_string()),
                    connect_ms: 0,
                    tx_started_at_ms: 0,
                    tx_finished_at_ms: 0,
                    recorded_samples: 0,
                    hangup_received: false,
                    analysis: None,
                    rx_wav_path: rx_wav_path.to_string_lossy().into_owned(),
                });
            }
        }
    }

    let summary = CampaignSummary::compute(
        campaign_id,
        config.campaign.name.clone(),
        backend_name,
        duration_ms,
        campaign_dir.to_string_lossy().into_owned(),
        call_results,
    );

    write_campaign_summary_json(&campaign_dir.join("campaign.json"), &summary)?;
    write_campaign_report_md(&campaign_dir.join("report.md"), &summary)?;

    Ok(summary)
}

fn call_outcome(recorded_samples: &[i16], analysis: &crate::analysis::CallAnalysis) -> String {
    match analysis.first_response_ms {
        Some(_) => "success".into(),
        None if recorded_samples.is_empty() => "no_audio".into(),
        None => "completed_without_response".into(),
    }
}

// ── Legacy single-call runner (Phase 1) ──────────────────────────────────────

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
