use common::config::load_config;
use voicebot_core::observability::{init_metrics, init_tracing};

#[tokio::main]
async fn main() {
    // Load config — fail fast on missing env vars or invalid config
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config.toml".into());

    let config = match load_config(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("FATAL: configuration error: {}", e);
            std::process::exit(1);
        }
    };
    // Init structured tracing
    init_tracing();

    tracing::info!("configuration loaded {:#?}", &config);

    // Init Prometheus metrics on port+1
    let metrics_addr = format!("{}:{}", config.server.host, config.server.port + 1);
    match init_metrics(&metrics_addr) {
        Ok(addr) => tracing::info!(%addr, "prometheus metrics server started"),
        Err(e) => {
            tracing::error!("failed to start metrics: {}", e);
            std::process::exit(1);
        }
    }

    // Extract listen address before moving config into Arc
    let listen_addr = format!("{}:{}", config.server.host, config.server.port);

    // Start ARI transport if configured.
    let asterisk_config = config.asterisk.clone();
    let config_arc = std::sync::Arc::new(config);

    if let Some(ari_cfg) = asterisk_config {
        let app_config = std::sync::Arc::clone(&config_arc);
        tokio::spawn(async move {
            tracing::info!(
                ari_host = %ari_cfg.ari_host,
                ari_port = ari_cfg.ari_port,
                "starting ARI transport"
            );
            let transport = transport_asterisk::AriTransport::new(ari_cfg, app_config);
            if let Err(e) = transport.run().await {
                tracing::error!("ARI transport error: {}", e);
            }
        });
    }

    // Build the WebSocket router with real providers from config
    let app = transport_websocket::handler::router_with_config(config_arc);

    let listener = match tokio::net::TcpListener::bind(&listen_addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(%listen_addr, "failed to bind: {}", e);
            std::process::exit(1);
        }
    };

    tracing::info!(%listen_addr, "voicebot server starting");

    // Graceful shutdown on SIGTERM / SIGINT
    let shutdown = async {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        let sigint = tokio::signal::ctrl_c();

        tokio::select! {
            _ = sigterm.recv() => tracing::info!("received SIGTERM"),
            _ = sigint => tracing::info!("received SIGINT"),
        }

        tracing::info!("initiating graceful shutdown (5s drain)");
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .expect("server error");

    tracing::info!("voicebot server stopped");
}
