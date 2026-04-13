//! Integration tests for the xphone SIP backend.
//!
//! These tests require a running Asterisk server with the `voicebot` endpoint
//! configured (see `system/asterisk/`). Run with:
//!
//!     cargo test -p voicebot-loadtest --test xphone_integration -- --ignored --nocapture

use voicebot_loadtest::config::LoadtestConfig;
use voicebot_loadtest::run_campaign;

#[tokio::test]
#[ignore]
async fn xphone_single_outbound_call() {
    let config: LoadtestConfig = toml::from_str(
        r#"
            [backend]
            kind = "xphone"

            [backend.xphone]
            sip_host = "localhost"
            sip_port = 5060
            username = "voicebot"
            password = "voicebot"
            rtp_port_min = 20000
            rtp_port_max = 20100
            register_timeout_ms = 10000
            call_timeout_ms = 30000

            [campaign]
            name = "xphone-integration-test"
            target_endpoint = "1000"
            total_calls = 1
            settle_before_playback_ms = 500
            record_after_playback_ms = 3000

            [media]
            input_wav = "tests/fixtures/audio/sample_speech.wav"
            artifact_dir = "artifacts/loadtest"
        "#,
    )
    .expect("config should parse");

    let summary = run_campaign(&config)
        .await
        .expect("campaign should complete");

    println!(
        "outcome: total={} success={}",
        summary.total_calls, summary.successful
    );
    println!("artifact_dir: {}", summary.artifact_dir);
    if let Some(p50) = summary.p50_first_response_ms {
        println!("first_response p50={}ms", p50);
    }
    assert_eq!(summary.total_calls, 1);
}

#[tokio::test]
#[ignore]
async fn xphone_concurrent_outbound_calls() {
    let config: LoadtestConfig = toml::from_str(
        r#"
            [backend]
            kind = "xphone"

            [backend.xphone]
            sip_host = "localhost"
            sip_port = 5060
            username = "voicebot"
            password = "voicebot"
            rtp_port_min = 20000
            rtp_port_max = 20100
            register_timeout_ms = 10000
            call_timeout_ms = 30000

            [campaign]
            name = "xphone-concurrent-test"
            target_endpoint = "1000"
            total_calls = 3
            concurrency = 2
            settle_before_playback_ms = 200
            record_after_playback_ms = 2000

            [media]
            input_wav = "tests/fixtures/audio/sample_speech.wav"
            artifact_dir = "artifacts/loadtest"
        "#,
    )
    .expect("config should parse");

    let summary = run_campaign(&config)
        .await
        .expect("campaign should complete");

    println!(
        "outcome: total={} success={} failed={}",
        summary.total_calls, summary.successful, summary.failed
    );
    println!("duration: {}ms", summary.duration_ms);
    assert_eq!(summary.total_calls, 3);
}

/// Inbound mode test: register with Asterisk, wait for it to call us.
///
/// Requires a running Asterisk that dials the registered `voicebot` endpoint.
/// For example, use AMI/ARI originate to call `PJSIP/voicebot` after starting
/// this test, or configure a periodic call in extensions.conf.
#[tokio::test]
#[ignore]
async fn xphone_single_inbound_call() {
    let config: LoadtestConfig = toml::from_str(
        r#"
            [backend]
            kind = "xphone"

            [backend.xphone]
            sip_host = "localhost"
            sip_port = 5060
            username = "voicebot"
            password = "voicebot"
            rtp_port_min = 20000
            rtp_port_max = 20100
            register_timeout_ms = 10000
            call_timeout_ms = 30000

            [campaign]
            name = "xphone-inbound-test"
            mode = "inbound"
            target_endpoint = "n/a"
            total_calls = 1
            inbound_timeout_ms = 60000
            settle_before_playback_ms = 500
            record_after_playback_ms = 3000

            [media]
            input_wav = "tests/fixtures/audio/sample_speech.wav"
            artifact_dir = "artifacts/loadtest"
        "#,
    )
    .expect("config should parse");

    let summary = run_campaign(&config)
        .await
        .expect("campaign should complete");

    println!(
        "inbound outcome: total={} success={}",
        summary.total_calls, summary.successful
    );
    assert_eq!(summary.total_calls, 1);
}
