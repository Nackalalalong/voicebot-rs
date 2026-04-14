use std::collections::BTreeMap;
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

    let _ = writeln!(md, "# Load Test Campaign Report\n");
    let _ = writeln!(md, "**Campaign:** {}  ", summary.campaign_name);
    let _ = writeln!(md, "**ID:** {}  ", summary.campaign_id);
    let _ = writeln!(md, "**Backend:** {}  ", summary.backend);
    let _ = writeln!(
        md,
        "**Duration:** {:.1}s  \n",
        summary.duration_ms as f64 / 1000.0
    );

    let _ = writeln!(md, "## Summary\n");
    let _ = writeln!(md, "| Metric | Value |");
    let _ = writeln!(md, "|---|---|");
    let _ = writeln!(md, "| Total calls | {} |", summary.total_calls);
    let _ = writeln!(md, "| Successful | {} |", summary.successful);
    let _ = writeln!(md, "| Failed | {} |", summary.failed);

    let _ = writeln!(
        md,
        "| P50 first response | {} |",
        opt_ms(summary.p50_first_response_ms)
    );
    let _ = writeln!(
        md,
        "| P90 first response | {} |",
        opt_ms(summary.p90_first_response_ms)
    );
    let _ = writeln!(
        md,
        "| P99 first response | {} |",
        opt_ms(summary.p99_first_response_ms)
    );
    let _ = writeln!(
        md,
        "| Avg first response | {} |",
        opt_ms(summary.avg_first_response_ms)
    );
    let _ = writeln!(
        md,
        "| Avg longest gap | {} ms |",
        summary.avg_longest_gap_ms
    );
    let _ = writeln!(
        md,
        "| Total stutter events | {} |",
        summary.total_stutter_count
    );

    let _ = writeln!(md, "\n## Call Results\n");
    let _ = writeln!(
        md,
        "| # | Outcome | Connect ms | First Resp ms | Longest Gap ms | Stutters | Samples |"
    );
    let _ = writeln!(md, "|---|---|---|---|---|---|---|");

    for call in &summary.call_results {
        let (first_resp, longest_gap, stutters) = match &call.analysis {
            Some(a) => (
                opt_ms(a.first_response_ms),
                a.longest_gap_ms.to_string(),
                a.stutter_count.to_string(),
            ),
            None => ("—".into(), "—".into(), "—".into()),
        };
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} | {} | {} |",
            call.call_index,
            call.outcome,
            call.connect_ms,
            first_resp,
            longest_gap,
            stutters,
            call.recorded_samples,
        );
    }

    std::fs::write(path, md)?;
    Ok(())
}

pub fn write_campaign_report_html(
    path: &Path,
    summary: &CampaignSummary,
) -> Result<(), LoadtestError> {
    let mut html = String::with_capacity(24 * 1024);
    let success_rate = percentage(summary.successful, summary.total_calls);
    let response_detected = summary
        .call_results
        .iter()
        .filter(|call| {
            call.analysis
                .as_ref()
                .and_then(|analysis| analysis.first_response_ms)
                .is_some()
        })
        .count();
    let no_audio_calls = summary
        .call_results
        .iter()
        .filter(|call| call.outcome == "no_audio")
        .count();
    let gap_issue_calls = summary
        .call_results
        .iter()
        .filter(|call| {
            call.analysis
                .as_ref()
                .is_some_and(|analysis| analysis.gap_count_over_threshold > 0)
        })
        .count();
    let stutter_issue_calls = summary
        .call_results
        .iter()
        .filter(|call| {
            call.analysis
                .as_ref()
                .is_some_and(|analysis| analysis.stutter_count > 0)
        })
        .count();
    let avg_voiced_share = average_voiced_share(summary);
    let outcome_rows = outcome_rows(summary);
    let outcome_max = outcome_rows
        .iter()
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(0)
        .max(1);
    let latency_bins = first_response_bins(summary);
    let latency_bin_max = latency_bins
        .iter()
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(0)
        .max(1);
    let longest_gap_bins = longest_gap_bins(summary);
    let longest_gap_bin_max = longest_gap_bins
        .iter()
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(0)
        .max(1);
    let analyzed_calls_for_gap_histogram = longest_gap_bins
        .iter()
        .map(|(_, count)| *count)
        .sum::<usize>();
    let all_gap_bins = all_gap_bins(summary);
    let all_gap_bin_max = all_gap_bins
        .iter()
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(0)
        .max(1);
    let total_gaps_observed = all_gap_bins.iter().map(|(_, count)| *count).sum::<usize>();
    let inspection_calls = inspection_calls(summary);

    html.push_str(
                r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Load Test Campaign Report</title>
    <style>
        :root { color-scheme: light; --bg: #f5efe6; --panel: rgba(255,255,255,0.82); --panel-strong: #fffdfa; --text: #1f2933; --muted: #5d6b78; --line: rgba(31,41,51,0.12); --accent: #bb4d00; --accent-soft: rgba(187,77,0,0.14); --good: #1f7a4d; --warn: #a16207; --bad: #b42318; --shadow: 0 18px 48px rgba(73, 46, 23, 0.12); }
        * { box-sizing: border-box; }
        body { margin: 0; font-family: "IBM Plex Sans", "Segoe UI", sans-serif; color: var(--text); background: radial-gradient(circle at top left, #fff5de 0%, var(--bg) 48%, #ebe3d7 100%); }
        a { color: var(--accent); }
        .page { max-width: 1280px; margin: 0 auto; padding: 32px 20px 48px; }
        .hero { padding: 28px; border: 1px solid rgba(255,255,255,0.45); border-radius: 28px; background: linear-gradient(135deg, rgba(255,253,250,0.95), rgba(255,244,230,0.92)); box-shadow: var(--shadow); }
        .eyebrow { margin: 0 0 10px; text-transform: uppercase; letter-spacing: 0.12em; font-size: 12px; color: var(--muted); }
        h1, h2, h3 { margin: 0; font-family: "IBM Plex Serif", "Georgia", serif; }
        h1 { font-size: clamp(2rem, 3.5vw, 3.4rem); line-height: 1.02; }
        h2 { font-size: 1.35rem; margin-bottom: 18px; }
        p { margin: 0; }
        .hero-meta { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 12px; margin-top: 18px; color: var(--muted); }
        .hero-meta strong { display: block; color: var(--text); font-size: 0.95rem; }
        .grid { display: grid; gap: 18px; margin-top: 18px; }
        .cards { grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); }
        .two-col { grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); }
        .panel { background: var(--panel); border: 1px solid rgba(255,255,255,0.42); border-radius: 24px; padding: 20px; box-shadow: var(--shadow); backdrop-filter: blur(18px); }
        .metric { display: flex; flex-direction: column; gap: 10px; min-height: 128px; }
        .metric-label { color: var(--muted); font-size: 0.92rem; }
        .metric-value { font-size: clamp(1.8rem, 3vw, 2.5rem); font-weight: 700; line-height: 1; }
        .metric-note { color: var(--muted); font-size: 0.92rem; }
        .outcome-row, .latency-row { display: grid; grid-template-columns: minmax(120px, 170px) 1fr auto; gap: 12px; align-items: center; margin-top: 12px; }
        .bar-track { height: 12px; border-radius: 999px; background: rgba(31,41,51,0.08); overflow: hidden; }
        .bar-fill { height: 100%; border-radius: inherit; background: linear-gradient(90deg, #d97706, var(--accent)); }
        .bar-fill.good { background: linear-gradient(90deg, #2e9b6d, var(--good)); }
        .bar-fill.warn { background: linear-gradient(90deg, #d0a33b, var(--warn)); }
        .bar-fill.bad { background: linear-gradient(90deg, #d8695c, var(--bad)); }
        .stat-list { display: grid; gap: 12px; }
        .stat-item { display: flex; justify-content: space-between; gap: 12px; padding: 12px 14px; border-radius: 16px; background: rgba(255,255,255,0.56); border: 1px solid rgba(31,41,51,0.06); }
        .muted { color: var(--muted); }
        table { width: 100%; border-collapse: collapse; font-size: 0.95rem; }
        th, td { padding: 10px 12px; text-align: left; border-bottom: 1px solid var(--line); vertical-align: top; }
        th { color: var(--muted); font-weight: 600; }
        td strong { display: inline-block; }
        .pill { display: inline-flex; align-items: center; border-radius: 999px; padding: 4px 10px; font-size: 0.82rem; font-weight: 600; background: var(--accent-soft); color: var(--accent); }
        .pill.success { background: rgba(31,122,77,0.14); color: var(--good); }
        .pill.warn { background: rgba(161,98,7,0.16); color: var(--warn); }
        .pill.bad { background: rgba(180,35,24,0.14); color: var(--bad); }
        details { margin-top: 18px; }
        summary { cursor: pointer; color: var(--accent); font-weight: 600; }
        @media (max-width: 720px) { .page { padding: 20px 14px 36px; } .hero { padding: 22px; border-radius: 22px; } .panel { padding: 18px; } .outcome-row, .latency-row { grid-template-columns: 1fr; } table { font-size: 0.88rem; } th:nth-child(4), td:nth-child(4), th:nth-child(7), td:nth-child(7) { display: none; } }
    </style>
</head>
<body>
    <main class="page">
"#,
        );

    let _ = write!(
        html,
        "    <section class=\"hero\">\n      <p class=\"eyebrow\">Loadtest artifact</p>\n      <h1>{}</h1>\n      <div class=\"hero-meta\">\n        <div><strong>Campaign ID</strong>{}</div>\n        <div><strong>Backend</strong>{}</div>\n        <div><strong>Total Calls</strong>{}</div>\n        <div><strong>Duration</strong>{:.1}s</div>\n      </div>\n    </section>\n",
        escape_html(&summary.campaign_name),
        escape_html(&summary.campaign_id),
        escape_html(&summary.backend),
        summary.total_calls,
        summary.duration_ms as f64 / 1000.0,
    );

    let _ = write!(html, "    <section class=\"grid cards\">\n");
    push_metric_card(
        &mut html,
        "Success Rate",
        &format!("{success_rate:.1}%"),
        &format!(
            "{} successful / {} failed",
            summary.successful, summary.failed
        ),
    );
    push_metric_card(
        &mut html,
        "P50 First Response",
        &opt_ms(summary.p50_first_response_ms),
        &format!("{} calls detected a response", response_detected),
    );
    push_metric_card(
        &mut html,
        "P90 First Response",
        &opt_ms(summary.p90_first_response_ms),
        &opt_ms_label("P99", summary.p99_first_response_ms),
    );
    push_metric_card(
        &mut html,
        "Average Longest Gap",
        &format!("{} ms", summary.avg_longest_gap_ms),
        &format!("{} total stutter events", summary.total_stutter_count),
    );
    push_metric_card(
        &mut html,
        "Gap-Heavy Calls",
        &gap_issue_calls.to_string(),
        "Calls with at least one response gap over threshold",
    );
    push_metric_card(
        &mut html,
        "Average Voiced Share",
        &format!("{avg_voiced_share:.1}%"),
        &format!("{} no-audio calls", no_audio_calls),
    );
    let _ = write!(html, "    </section>\n");

    let _ = write!(html, "    <section class=\"grid two-col\">\n");
    let _ = write!(
        html,
        "      <section class=\"panel\">\n        <h2>Outcome Mix</h2>\n        <p class=\"muted\">The distribution below makes failure modes visible without opening individual call artifacts.</p>\n"
    );
    for (label, count) in &outcome_rows {
        let width = (*count as f64 * 100.0) / outcome_max as f64;
        let pill_class = outcome_class(label);
        let _ = write!(
            html,
            "        <div class=\"outcome-row\">\n          <div>{}</div>\n          <div class=\"bar-track\"><div class=\"bar-fill {}\" style=\"width:{:.1}%\"></div></div>\n          <div>{} ({:.1}%)</div>\n        </div>\n",
            escape_html(label),
            pill_class,
            width,
            count,
            percentage(*count, summary.total_calls),
        );
    }
    let _ = write!(html, "      </section>\n");

    let _ = write!(
        html,
        "      <section class=\"panel\">\n        <h2>First Response Latency</h2>\n        <p class=\"muted\">Bucketed first-response times show whether the run is dominated by fast starts or a long slow tail.</p>\n"
    );
    for (label, count) in &latency_bins {
        let width = (*count as f64 * 100.0) / latency_bin_max as f64;
        let _ = write!(
            html,
            "        <div class=\"latency-row\">\n          <div>{}</div>\n          <div class=\"bar-track\"><div class=\"bar-fill\" style=\"width:{:.1}%\"></div></div>\n          <div>{}</div>\n        </div>\n",
            escape_html(label),
            width,
            count,
        );
    }
    let _ = write!(html, "      </section>\n    </section>\n");

    let _ = write!(html, "    <section class=\"grid two-col\">\n");
    let _ = write!(
        html,
        "      <section class=\"panel\">\n        <h2>Longest Gap Histogram</h2>\n        <p class=\"muted\">Each bar counts analyzed calls by their single worst silence gap after speech begins.</p>\n        <div class=\"stat-item\"><span>Analyzed calls</span><strong>{}</strong></div>\n",
        analyzed_calls_for_gap_histogram,
    );
    for (label, count) in &longest_gap_bins {
        let width = (*count as f64 * 100.0) / longest_gap_bin_max as f64;
        let _ = write!(
            html,
            "        <div class=\"latency-row\">\n          <div>{}</div>\n          <div class=\"bar-track\"><div class=\"bar-fill warn\" style=\"width:{:.1}%\"></div></div>\n          <div>{}</div>\n        </div>\n",
            escape_html(label),
            width,
            count,
        );
    }
    let _ = write!(html, "      </section>\n");

    let _ = write!(
        html,
        "      <section class=\"panel\">\n        <h2>All Gap Histogram</h2>\n        <p class=\"muted\">Every inter-voiced gap across the run. This distinguishes a few bad outliers from a broadly choppy response profile.</p>\n        <div class=\"stat-item\"><span>Total gaps observed</span><strong>{}</strong></div>\n",
        total_gaps_observed,
    );
    for (label, count) in &all_gap_bins {
        let width = (*count as f64 * 100.0) / all_gap_bin_max as f64;
        let _ = write!(
            html,
            "        <div class=\"latency-row\">\n          <div>{}</div>\n          <div class=\"bar-track\"><div class=\"bar-fill bad\" style=\"width:{:.1}%\"></div></div>\n          <div>{}</div>\n        </div>\n",
            escape_html(label),
            width,
            count,
        );
    }
    let _ = write!(html, "      </section>\n    </section>\n");

    let _ = write!(html, "    <section class=\"grid two-col\">\n");
    let _ = write!(
        html,
        "      <section class=\"panel\">\n        <h2>Quality Signals</h2>\n        <div class=\"stat-list\">\n"
    );
    push_stat_item(
        &mut html,
        "Detected responses",
        &format!("{} / {} calls", response_detected, summary.total_calls),
    );
    push_stat_item(
        &mut html,
        "Stutter-prone calls",
        &format!("{} calls", stutter_issue_calls),
    );
    push_stat_item(
        &mut html,
        "Calls without audio",
        &format!("{} calls", no_audio_calls),
    );
    push_stat_item(
        &mut html,
        "Artifact directory",
        &escape_html(&summary.artifact_dir),
    );
    let _ = write!(html, "        </div>\n      </section>\n");

    let _ = write!(
        html,
        "      <section class=\"panel\">\n        <h2>Calls To Inspect</h2>\n        <p class=\"muted\">Sorted so failures and the slowest or choppiest responses rise to the top.</p>\n        <table>\n          <thead><tr><th>#</th><th>Outcome</th><th>First Response</th><th>Longest Gap</th><th>Stutters</th><th>Audio</th></tr></thead>\n          <tbody>\n"
    );
    for call in inspection_calls {
        let analysis = call.analysis.as_ref();
        let pill_class = pill_class(&call.outcome);
        let _ = write!(
            html,
            "            <tr><td>{}</td><td><span class=\"pill {}\">{}</span></td><td>{}</td><td>{}</td><td>{}</td><td><a href=\"{}\">rx.wav</a></td></tr>\n",
            call.call_index,
            pill_class,
            escape_html(&call.outcome),
            opt_ms(analysis.and_then(|item| item.first_response_ms)),
            analysis
                .map(|item| format!("{} ms", item.longest_gap_ms))
                .unwrap_or_else(|| "—".into()),
            analysis
                .map(|item| item.stutter_count.to_string())
                .unwrap_or_else(|| "—".into()),
            escape_html(&relative_artifact_path(summary, &call.rx_wav_path)),
        );
    }
    let _ = write!(
        html,
        "          </tbody>\n        </table>\n      </section>\n    </section>\n"
    );

    let _ = write!(
        html,
        "    <section class=\"panel\">\n      <h2>Per-Call Detail</h2>\n      <p class=\"muted\">Full call list for correlation with raw audio artifacts and campaign JSON.</p>\n      <details open>\n        <summary>Show all calls</summary>\n        <table>\n          <thead><tr><th>#</th><th>Outcome</th><th>Connect</th><th>First Response</th><th>Longest Gap</th><th>Voiced Share</th><th>Samples</th><th>Error</th><th>Audio</th></tr></thead>\n          <tbody>\n"
    );

    for call in &summary.call_results {
        let analysis = call.analysis.as_ref();
        let pill_class = pill_class(&call.outcome);
        let voiced_share = analysis
            .and_then(voiced_share)
            .map(|share| format!("{share:.1}%"))
            .unwrap_or_else(|| "—".into());
        let error = call
            .error
            .as_deref()
            .map(escape_html)
            .unwrap_or_else(|| "—".into());
        let _ = write!(
            html,
            "            <tr><td>{}</td><td><span class=\"pill {}\">{}</span></td><td>{} ms</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><a href=\"{}\">rx.wav</a></td></tr>\n",
            call.call_index,
            pill_class,
            escape_html(&call.outcome),
            call.connect_ms,
            opt_ms(analysis.and_then(|item| item.first_response_ms)),
            analysis
                .map(|item| format!("{} ms", item.longest_gap_ms))
                .unwrap_or_else(|| "—".into()),
            voiced_share,
            call.recorded_samples,
            error,
            escape_html(&relative_artifact_path(summary, &call.rx_wav_path)),
        );
    }

    let _ = write!(
        html,
        "          </tbody>\n        </table>\n      </details>\n    </section>\n  </main>\n</body>\n</html>\n"
    );

    std::fs::write(path, html)?;
    Ok(())
}

fn opt_ms(v: Option<u64>) -> String {
    match v {
        Some(ms) => format!("{} ms", ms),
        None => "—".into(),
    }
}

fn opt_ms_label(label: &str, value: Option<u64>) -> String {
    format!("{label}: {}", opt_ms(value))
}

fn percentage(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        (count as f64 * 100.0) / total as f64
    }
}

fn average_voiced_share(summary: &CampaignSummary) -> f64 {
    let shares: Vec<f64> = summary
        .call_results
        .iter()
        .filter_map(|call| call.analysis.as_ref().and_then(voiced_share))
        .collect();

    if shares.is_empty() {
        0.0
    } else {
        shares.iter().sum::<f64>() / shares.len() as f64
    }
}

fn voiced_share(analysis: &CallAnalysis) -> Option<f64> {
    if analysis.recorded_duration_ms == 0 {
        None
    } else {
        Some((analysis.voiced_duration_ms as f64 * 100.0) / analysis.recorded_duration_ms as f64)
    }
}

fn outcome_rows(summary: &CampaignSummary) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for call in &summary.call_results {
        *counts.entry(call.outcome.clone()).or_default() += 1;
    }

    let mut rows = Vec::new();
    for label in [
        "success",
        "completed_without_response",
        "no_audio",
        "failed",
    ] {
        if let Some(count) = counts.remove(label) {
            rows.push((label.to_string(), count));
        }
    }
    rows.extend(counts);
    rows
}

fn first_response_bins(summary: &CampaignSummary) -> Vec<(String, usize)> {
    let mut bins = vec![
        ("0-250 ms".to_string(), 0usize),
        ("251-500 ms".to_string(), 0usize),
        ("501-1000 ms".to_string(), 0usize),
        ("1001-2000 ms".to_string(), 0usize),
        ("> 2000 ms".to_string(), 0usize),
        ("No response".to_string(), 0usize),
    ];

    for call in &summary.call_results {
        match call
            .analysis
            .as_ref()
            .and_then(|analysis| analysis.first_response_ms)
        {
            Some(ms) if ms <= 250 => bins[0].1 += 1,
            Some(ms) if ms <= 500 => bins[1].1 += 1,
            Some(ms) if ms <= 1000 => bins[2].1 += 1,
            Some(ms) if ms <= 2000 => bins[3].1 += 1,
            Some(_) => bins[4].1 += 1,
            None => bins[5].1 += 1,
        }
    }

    bins
}

fn longest_gap_bins(summary: &CampaignSummary) -> Vec<(String, usize)> {
    let mut bins = vec![
        ("No gap".to_string(), 0usize),
        ("1-250 ms".to_string(), 0usize),
        ("251-500 ms".to_string(), 0usize),
        ("501-1000 ms".to_string(), 0usize),
        ("1001-2000 ms".to_string(), 0usize),
        ("2001-5000 ms".to_string(), 0usize),
        ("> 5000 ms".to_string(), 0usize),
    ];

    for call in &summary.call_results {
        let Some(longest_gap_ms) = call
            .analysis
            .as_ref()
            .map(|analysis| analysis.longest_gap_ms)
        else {
            continue;
        };

        match longest_gap_ms {
            0 => bins[0].1 += 1,
            1..=250 => bins[1].1 += 1,
            251..=500 => bins[2].1 += 1,
            501..=1000 => bins[3].1 += 1,
            1001..=2000 => bins[4].1 += 1,
            2001..=5000 => bins[5].1 += 1,
            _ => bins[6].1 += 1,
        }
    }

    bins
}

fn all_gap_bins(summary: &CampaignSummary) -> Vec<(String, usize)> {
    let mut bins = vec![
        ("1-250 ms".to_string(), 0usize),
        ("251-500 ms".to_string(), 0usize),
        ("501-1000 ms".to_string(), 0usize),
        ("1001-2000 ms".to_string(), 0usize),
        ("2001-5000 ms".to_string(), 0usize),
        ("> 5000 ms".to_string(), 0usize),
    ];

    for gap_ms in all_gap_values(summary) {
        match gap_ms {
            1..=250 => bins[0].1 += 1,
            251..=500 => bins[1].1 += 1,
            501..=1000 => bins[2].1 += 1,
            1001..=2000 => bins[3].1 += 1,
            2001..=5000 => bins[4].1 += 1,
            _ => bins[5].1 += 1,
        }
    }

    bins
}

fn all_gap_values(summary: &CampaignSummary) -> Vec<u64> {
    let mut gaps = Vec::new();

    for call in &summary.call_results {
        let Some(analysis) = &call.analysis else {
            continue;
        };

        for pair in analysis.voiced_regions.windows(2) {
            let previous = &pair[0];
            let next = &pair[1];
            let gap_ms = next.start_ms.saturating_sub(previous.end_ms);
            if gap_ms > 0 {
                gaps.push(gap_ms);
            }
        }
    }

    gaps
}

fn inspection_calls(summary: &CampaignSummary) -> Vec<&CallResult> {
    let mut calls: Vec<&CallResult> = summary.call_results.iter().collect();
    calls.sort_by(|left, right| inspection_rank(right).cmp(&inspection_rank(left)));
    calls.truncate(calls.len().min(8));
    calls
}

fn inspection_rank(call: &CallResult) -> (u8, u64, u64, u32) {
    let outcome_rank = match call.outcome.as_str() {
        "failed" => 4,
        "no_audio" => 3,
        "completed_without_response" => 2,
        "success" => 1,
        _ => 0,
    };
    let first_response_ms = call
        .analysis
        .as_ref()
        .and_then(|analysis| analysis.first_response_ms)
        .unwrap_or(u64::MAX);
    let longest_gap_ms = call
        .analysis
        .as_ref()
        .map(|analysis| analysis.longest_gap_ms)
        .unwrap_or(0);
    let stutter_count = call
        .analysis
        .as_ref()
        .map(|analysis| analysis.stutter_count)
        .unwrap_or(0);
    (
        outcome_rank,
        first_response_ms,
        longest_gap_ms,
        stutter_count,
    )
}

fn outcome_class(label: &str) -> &'static str {
    match label {
        "success" => "good",
        "completed_without_response" | "no_audio" => "warn",
        _ => "bad",
    }
}

fn pill_class(label: &str) -> &'static str {
    match label {
        "success" => "success",
        "completed_without_response" | "no_audio" => "warn",
        _ => "bad",
    }
}

fn relative_artifact_path(summary: &CampaignSummary, full_path: &str) -> String {
    let artifact_dir = Path::new(&summary.artifact_dir);
    let full_path = Path::new(full_path);
    full_path
        .strip_prefix(artifact_dir)
        .unwrap_or(full_path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn escape_html(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn push_metric_card(html: &mut String, label: &str, value: &str, note: &str) {
    let _ = write!(
        html,
        "      <section class=\"panel metric\"><div class=\"metric-label\">{}</div><div class=\"metric-value\">{}</div><div class=\"metric-note\">{}</div></section>\n",
        escape_html(label),
        escape_html(value),
        escape_html(note),
    );
}

fn push_stat_item(html: &mut String, label: &str, value: &str) {
    let _ = write!(
        html,
        "          <div class=\"stat-item\"><span>{}</span><strong>{}</strong></div>\n",
        escape_html(label),
        escape_html(value),
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn sample_analysis() -> CallAnalysis {
        CallAnalysis {
            recorded_duration_ms: 1_200,
            voiced_duration_ms: 600,
            silence_duration_ms: 600,
            first_response_ms: Some(320),
            longest_gap_ms: 280,
            gap_count_over_threshold: 1,
            stutter_count: 2,
            voiced_regions: vec![
                crate::analysis::VoicedRegion {
                    start_ms: 320,
                    end_ms: 700,
                },
                crate::analysis::VoicedRegion {
                    start_ms: 980,
                    end_ms: 1_200,
                },
            ],
        }
    }

    #[test]
    fn writes_html_report_with_visual_sections() -> Result<(), LoadtestError> {
        let output = NamedTempFile::new()?;
        let artifact_dir = "/tmp/loadtest-run".to_string();
        let summary = CampaignSummary::compute(
            "20260414_010203".into(),
            "Smoke Campaign".into(),
            "xphone".into(),
            4_200,
            artifact_dir.clone(),
            vec![
                CallResult {
                    call_index: 0,
                    outcome: "success".into(),
                    error: None,
                    connect_ms: 110,
                    tx_started_at_ms: 0,
                    tx_finished_at_ms: 900,
                    recorded_samples: 18_000,
                    hangup_received: true,
                    analysis: Some(sample_analysis()),
                    rx_wav_path: format!("{artifact_dir}/calls/0000/rx.wav"),
                },
                CallResult {
                    call_index: 1,
                    outcome: "failed".into(),
                    error: Some("controller timeout".into()),
                    connect_ms: 0,
                    tx_started_at_ms: 0,
                    tx_finished_at_ms: 0,
                    recorded_samples: 0,
                    hangup_received: false,
                    analysis: None,
                    rx_wav_path: format!("{artifact_dir}/calls/0001/rx.wav"),
                },
            ],
        );

        write_campaign_report_html(output.path(), &summary)?;

        let html = std::fs::read_to_string(output.path())?;
        assert!(html.contains("Load Test Campaign Report"));
        assert!(html.contains("Outcome Mix"));
        assert!(html.contains("First Response Latency"));
        assert!(html.contains("Longest Gap Histogram"));
        assert!(html.contains("All Gap Histogram"));
        assert!(html.contains("Calls To Inspect"));
        assert!(html.contains("calls/0000/rx.wav"));
        assert!(html.contains("controller timeout"));
        Ok(())
    }
}
