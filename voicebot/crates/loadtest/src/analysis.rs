use serde::Serialize;

use crate::config::AnalysisConfig;

const SAMPLE_RATE: u64 = 16_000;

#[derive(Debug, Clone, Serialize)]
pub struct CallAnalysis {
    pub recorded_duration_ms: u64,
    pub voiced_duration_ms: u64,
    pub silence_duration_ms: u64,
    pub first_response_ms: Option<u64>,
    pub longest_gap_ms: u64,
    pub gap_count_over_threshold: u32,
    /// Short gaps (< stutter_gap_ms) between consecutive voiced regions — indicates choppy audio.
    pub stutter_count: u32,
    pub voiced_regions: Vec<VoicedRegion>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VoicedRegion {
    pub start_ms: u64,
    pub end_ms: u64,
}

pub fn analyze_received_audio(
    samples: &[i16],
    tx_finished_at_ms: u64,
    config: &AnalysisConfig,
) -> CallAnalysis {
    let recorded_duration_ms = (samples.len() as u64 * 1000) / SAMPLE_RATE;
    let window_samples = ((SAMPLE_RATE * config.window_ms as u64) / 1000).max(1) as usize;

    let mut voiced_regions = Vec::new();
    let mut open_start_ms = None;

    for (index, chunk) in samples.chunks(window_samples).enumerate() {
        let start_ms = index as u64 * config.window_ms as u64;
        let end_ms = (start_ms + config.window_ms as u64).min(recorded_duration_ms);

        if is_voiced(chunk, config.silence_threshold) {
            if open_start_ms.is_none() {
                open_start_ms = Some(start_ms);
            }
        } else if let Some(region_start_ms) = open_start_ms.take() {
            voiced_regions.push(VoicedRegion {
                start_ms: region_start_ms,
                end_ms,
            });
        }
    }

    if let Some(region_start_ms) = open_start_ms {
        voiced_regions.push(VoicedRegion {
            start_ms: region_start_ms,
            end_ms: recorded_duration_ms,
        });
    }

    let voiced_duration_ms = voiced_regions
        .iter()
        .map(|region| region.end_ms.saturating_sub(region.start_ms))
        .sum::<u64>()
        .min(recorded_duration_ms);
    let silence_duration_ms = recorded_duration_ms.saturating_sub(voiced_duration_ms);

    let first_response_ms = voiced_regions.iter().find_map(|region| {
        if region.end_ms <= tx_finished_at_ms {
            None
        } else if region.start_ms <= tx_finished_at_ms {
            Some(0)
        } else {
            Some(region.start_ms.saturating_sub(tx_finished_at_ms))
        }
    });

    let mut longest_gap_ms = 0;
    let mut gap_count_over_threshold = 0;
    let mut stutter_count = 0u32;
    for pair in voiced_regions.windows(2) {
        let previous = &pair[0];
        let next = &pair[1];
        let gap_ms = next.start_ms.saturating_sub(previous.end_ms);
        if gap_ms > longest_gap_ms {
            longest_gap_ms = gap_ms;
        }
        if gap_ms >= config.gap_threshold_ms {
            gap_count_over_threshold += 1;
        }
        if gap_ms > 0 && gap_ms < config.stutter_gap_ms {
            stutter_count += 1;
        }
    }

    CallAnalysis {
        recorded_duration_ms,
        voiced_duration_ms,
        silence_duration_ms,
        first_response_ms,
        longest_gap_ms,
        gap_count_over_threshold,
        stutter_count,
        voiced_regions,
    }
}

fn is_voiced(samples: &[i16], threshold: f32) -> bool {
    if samples.is_empty() {
        return false;
    }
    let sum_sq = samples
        .iter()
        .map(|sample| (*sample as f64).powi(2))
        .sum::<f64>();
    let rms = (sum_sq / samples.len() as f64).sqrt() / i16::MAX as f64;
    rms > threshold as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(duration_ms: u64, amplitude: i16) -> Vec<i16> {
        let samples = (SAMPLE_RATE * duration_ms / 1000) as usize;
        vec![amplitude; samples]
    }

    fn silence(duration_ms: u64) -> Vec<i16> {
        let samples = (SAMPLE_RATE * duration_ms / 1000) as usize;
        vec![0; samples]
    }

    #[test]
    fn computes_first_response_and_gap_metrics() {
        let mut recorded = silence(300);
        recorded.extend(tone(200, 6_000));
        recorded.extend(silence(400));
        recorded.extend(tone(100, 6_000));

        let analysis = analyze_received_audio(
            &recorded,
            150,
            &AnalysisConfig {
                silence_threshold: 0.02,
                window_ms: 20,
                gap_threshold_ms: 250,
                stutter_gap_ms: 200,
            },
        );

        assert_eq!(analysis.first_response_ms, Some(150));
        assert!(analysis.longest_gap_ms >= 380);
        assert_eq!(analysis.gap_count_over_threshold, 1);
    }
}
