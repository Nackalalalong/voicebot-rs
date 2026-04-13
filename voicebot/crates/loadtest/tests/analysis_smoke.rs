use voicebot_loadtest::analysis::analyze_received_audio;
use voicebot_loadtest::config::AnalysisConfig;

fn tone(duration_ms: u64, amplitude: i16) -> Vec<i16> {
    let samples = (16_000 * duration_ms / 1000) as usize;
    vec![amplitude; samples]
}

fn silence(duration_ms: u64) -> Vec<i16> {
    let samples = (16_000 * duration_ms / 1000) as usize;
    vec![0; samples]
}

#[test]
fn analysis_detects_missing_response() {
    let recorded = silence(1000);
    let analysis = analyze_received_audio(
        &recorded,
        200,
        &AnalysisConfig {
            silence_threshold: 0.02,
            window_ms: 20,
            gap_threshold_ms: 250,
            stutter_gap_ms: 200,
        },
    );

    assert_eq!(analysis.first_response_ms, None);
    assert_eq!(analysis.voiced_duration_ms, 0);
}

#[test]
fn analysis_detects_response_after_tx_finishes() {
    let mut recorded = tone(150, 5_000);
    recorded.extend(silence(200));
    recorded.extend(tone(100, 5_000));

    let analysis = analyze_received_audio(
        &recorded,
        100,
        &AnalysisConfig {
            silence_threshold: 0.02,
            window_ms: 20,
            gap_threshold_ms: 150,
            stutter_gap_ms: 200,
        },
    );

    assert_eq!(analysis.first_response_ms, Some(0));
    assert!(analysis.gap_count_over_threshold >= 1);
}
