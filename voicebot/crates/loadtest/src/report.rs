use std::fmt::Write as FmtWrite;
use std::path::Path;

use serde::Serialize;

use crate::analysis::CallAnalysis;
use crate::error::LoadtestError;

// ── Per-call result (used in campaign reports) ────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct CallResult {
    pub call_index: usize,
    /// "success" | "no_audio" | "completed_without_response" | "failed"
    pub outcome: String,
    pub error: Option<String>,
    pub connect_ms: u64,
    pub tx_started_at_ms: u64,
    pub tx_finished_at_ms: u64,
    pub recorded_samples: usize,
    pub hangup_received: bool,
    pub analysis: Option<CallAnalysis>,
    pub rx_wav_path: String,
}

// ── Aggregated campaign summary ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct CampaignSummary {
    pub campaign_id: String,
    pub campaign_name: String,
    pub backend: String,
    pub total_calls: usize,
    pub successful: usize,
    pub failed: usize,
    pub duration_ms: u64,
    pub p50_first_response_ms: Option<u64>,
    pub p90_first_response_ms: Option<u64>,
    pub p99_first_response_ms: Option<u64>,
    pub avg_first_response_ms: Option<u64>,
    pub avg_longest_gap_ms: u64,
    pub total_stutter_count: u32,
    pub artifact_dir: String,
    pub call_results: Vec<CallResult>,
}

impl CampaignSummary {
    pub fn compute(
        campaign_id: String,
        campaign_name: String,
        backend: String,
        duration_ms: u64,
        artifact_dir: String,
        calls: Vec<CallResult>,
    ) -> Self {
        let total_calls = calls.len();
        let successful = calls.iter().filter(|c| c.outcome == "success").count();
        let failed = total_calls - successful;

        let mut first_responses: Vec<u64> = calls
            .iter()
            .filter_map(|c| c.analysis.as_ref()?.first_response_ms)
            .collect();
        first_responses.sort_unstable();

        let p50 = percentile(&first_responses, 50);
        let p90 = percentile(&first_responses, 90);
        let p99 = percentile(&first_responses, 99);
        let avg_first = if first_responses.is_empty() {
            None
        } else {
            Some(first_responses.iter().sum::<u64>() / first_responses.len() as u64)
        };

        let gap_sum: u64 = calls
            .iter()
            .filter_map(|c| c.analysis.as_ref())
            .map(|a| a.longest_gap_ms)
            .sum();
        let avg_longest_gap_ms = if total_calls > 0 {
            gap_sum / total_calls as u64
        } else {
            0
        };

        let total_stutter_count: u32 = calls
            .iter()
            .filter_map(|c| c.analysis.as_ref())
            .map(|a| a.stutter_count)
            .sum();

        Self {
            campaign_id,
            campaign_name,
            backend,
            total_calls,
            successful,
            failed,
            duration_ms,
            p50_first_response_ms: p50,
            p90_first_response_ms: p90,
            p99_first_response_ms: p99,
            avg_first_response_ms: avg_first,
            avg_longest_gap_ms,
            total_stutter_count,
            artifact_dir,
            call_results: calls,
        }
    }
}

fn percentile(sorted: &[u64], pct: usize) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let index = ((sorted.len() - 1) * pct) / 100;
    Some(sorted[index])
}

pub fn write_campaign_summary_json(
    path: &Path,
    summary: &CampaignSummary,
) -> Result<(), LoadtestError> {
    let json = serde_json::to_string_pretty(summary)?;
    std::fs::write(path, json)?;
    Ok(())
}

pub fn write_campaign_report_md(
    path: &Path,
    summary: &CampaignSummary,
) -> Result<(), LoadtestError> {
    let mut md = String::new();

    writeln!(md, "# Load Test Campaign Report\n").unwrap();
    writeln!(md, "**Campaign:** {}  ", summary.campaign_name).unwrap();
    writeln!(md, "**ID:** {}  ", summary.campaign_id).unwrap();
    writeln!(md, "**Backend:** {}  ", summary.backend).unwrap();
    writeln!(
        md,
        "**Duration:** {:.1}s  \n",
        summary.duration_ms as f64 / 1000.0
    )
    .unwrap();

    writeln!(md, "## Summary\n").unwrap();
    writeln!(md, "| Metric | Value |").unwrap();
    writeln!(md, "|---|---|").unwrap();
    writeln!(md, "| Total calls | {} |", summary.total_calls).unwrap();
    writeln!(md, "| Successful | {} |", summary.successful).unwrap();
    writeln!(md, "| Failed | {} |", summary.failed).unwrap();

    writeln!(
        md,
        "| P50 first response | {} |",
        opt_ms(summary.p50_first_response_ms)
    )
    .unwrap();
    writeln!(
        md,
        "| P90 first response | {} |",
        opt_ms(summary.p90_first_response_ms)
    )
    .unwrap();
    writeln!(
        md,
        "| P99 first response | {} |",
        opt_ms(summary.p99_first_response_ms)
    )
    .unwrap();
    writeln!(
        md,
        "| Avg first response | {} |",
        opt_ms(summary.avg_first_response_ms)
    )
    .unwrap();
    writeln!(
        md,
        "| Avg longest gap | {} ms |",
        summary.avg_longest_gap_ms
    )
    .unwrap();
    writeln!(
        md,
        "| Total stutter events | {} |",
        summary.total_stutter_count
    )
    .unwrap();

    writeln!(md, "\n## Call Results\n").unwrap();
    writeln!(
        md,
        "| # | Outcome | Connect ms | First Resp ms | Longest Gap ms | Stutters | Samples |"
    )
    .unwrap();
    writeln!(md, "|---|---|---|---|---|---|---|").unwrap();

    for call in &summary.call_results {
        let (first_resp, longest_gap, stutters) = match &call.analysis {
            Some(a) => (
                opt_ms(a.first_response_ms),
                a.longest_gap_ms.to_string(),
                a.stutter_count.to_string(),
            ),
            None => ("—".into(), "—".into(), "—".into()),
        };
        writeln!(
            md,
            "| {} | {} | {} | {} | {} | {} | {} |",
            call.call_index,
            call.outcome,
            call.connect_ms,
            first_resp,
            longest_gap,
            stutters,
            call.recorded_samples,
        )
        .unwrap();
    }

    std::fs::write(path, md)?;
    Ok(())
}

fn opt_ms(v: Option<u64>) -> String {
    match v {
        Some(ms) => format!("{} ms", ms),
        None => "—".into(),
    }
}

// ── Legacy single-run summary ─────────────────────────────────────────────────

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
