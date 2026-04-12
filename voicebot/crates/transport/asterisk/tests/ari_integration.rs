use std::time::Duration;

use common::config::AsteriskConfig;
use futures::{SinkExt, StreamExt};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use transport_asterisk::AriRestClient;

fn test_asterisk_config() -> AsteriskConfig {
    AsteriskConfig {
        ari_host: std::env::var("ASTERISK_ARI_HOST").unwrap_or_else(|_| "localhost".into()),
        ari_port: std::env::var("ASTERISK_ARI_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(8088),
        username: std::env::var("ASTERISK_ARI_USERNAME").unwrap_or_else(|_| "voicebot".into()),
        password: std::env::var("ASTERISK_ARI_PASSWORD").unwrap_or_else(|_| "voicebot".into()),
        app_name: std::env::var("ASTERISK_ARI_APP").unwrap_or_else(|_| "voicebot".into()),
        audio_host: std::env::var("ASTERISK_AUDIO_HOST").unwrap_or_else(|_| "172.17.0.1".into()),
    }
}

#[tokio::test]
#[ignore = "requires running Asterisk server"]
async fn test_ari_rest_client_reads_asterisk_info() {
    let client = AriRestClient::new(&test_asterisk_config());
    let info = client
        .asterisk_info()
        .await
        .expect("ARI info request failed");

    assert_eq!(info["build"]["os"], "Linux");
    assert!(info["system"]["version"].as_str().is_some());
}

#[tokio::test]
#[ignore = "requires running Asterisk server"]
async fn test_ari_reports_voicebot_endpoint_status() {
    let client = AriRestClient::new(&test_asterisk_config());
    let endpoint = client
        .endpoint("PJSIP", "voicebot")
        .await
        .expect("endpoint lookup failed");

    assert_eq!(endpoint["technology"], "PJSIP");
    assert_eq!(endpoint["resource"], "voicebot");
    assert!(endpoint["state"].as_str().is_some());
    assert!(endpoint["channel_ids"].as_array().is_some());
}

#[tokio::test]
#[ignore = "requires running Asterisk server"]
async fn test_ari_websocket_accepts_ping_and_returns_pong() {
    let config = test_asterisk_config();
    let ws_url = format!(
        "ws://{}:{}/ari/events?api_key={}:{}&app={}&subscribeAll=true",
        config.ari_host, config.ari_port, config.username, config.password, config.app_name,
    );
    let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("failed to connect to ARI websocket");
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    ws_tx
        .send(Message::Ping(Vec::new().into()))
        .await
        .expect("failed to send websocket ping");

    let wait_for_pong = async {
        loop {
            match ws_rx.next().await {
                Some(Ok(Message::Pong(_))) => return,
                Some(Ok(Message::Ping(payload))) => {
                    ws_tx
                        .send(Message::Pong(payload))
                        .await
                        .expect("failed to respond to server ping");
                }
                Some(Ok(Message::Text(_))) => {
                    continue;
                }
                Some(Ok(Message::Binary(_))) => {
                    continue;
                }
                Some(Ok(Message::Frame(_))) => {
                    continue;
                }
                Some(Ok(Message::Close(_))) => {
                    panic!("ARI websocket closed before Pong")
                }
                Some(Err(err)) => panic!("ARI websocket failed: {err}"),
                None => panic!("ARI websocket closed before receiving Pong"),
            }
        }
    };

    timeout(Duration::from_secs(10), wait_for_pong)
        .await
        .expect("timed out waiting for websocket Pong");

    ws_tx
        .close()
        .await
        .expect("failed to close websocket cleanly");
}

#[tokio::test]
#[ignore = "requires running Asterisk server"]
async fn test_ari_rest_client_can_originate_and_hangup_local_channel() {
    let config = test_asterisk_config();
    let client = AriRestClient::new(&config);

    let channel_id = client
        .originate_in_app(
            "Local/1000@dp_entry_call_in",
            &config.app_name,
            "ari-integration-test",
        )
        .await
        .expect("failed to originate local channel");

    assert!(!channel_id.is_empty());

    client
        .hangup_channel(&channel_id)
        .await
        .expect("failed to hang up originated channel");
}
