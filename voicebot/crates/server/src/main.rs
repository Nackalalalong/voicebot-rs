use std::sync::Arc;

use common::config::load_config;
use transport_websocket::handler::PlatformContext;
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
    let config_arc = Arc::new(config);

    if let Some(ari_cfg) = asterisk_config {
        let app_config = Arc::clone(&config_arc);
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

    // Optionally connect to DB + Redis for platform features (JWT auth, CDR, session tracking).
    // Falls back to config-only mode when env vars are absent.
    let app = match build_platform_context(Arc::clone(&config_arc)).await {
        Some(platform) => {
            tracing::info!("platform context ready — using authenticated router");
            transport_websocket::handler::router_with_platform(config_arc, Arc::new(platform))
        }
        None => {
            tracing::info!("no platform credentials — using unauthenticated router");
            transport_websocket::handler::router_with_config(config_arc)
        }
    };

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

/// Attempt to connect to DB and Redis using env vars.
/// Returns None if DATABASE_URL, REDIS_URL, or JWT_SECRET are not set.
async fn build_platform_context(config: Arc<common::config::AppConfig>) -> Option<PlatformContext> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let redis_url = std::env::var("REDIS_URL").ok()?;
    let jwt_secret = std::env::var("JWT_SECRET").ok()?;

    let db = match db::connect(&database_url).await {
        Ok(pool) => pool,
        Err(e) => {
            tracing::warn!(error = %e, "failed to connect to DB — disabling platform features");
            return None;
        }
    };

    let redis = match cache::connect(&redis_url).await {
        Ok(pool) => pool,
        Err(e) => {
            tracing::warn!(error = %e, "failed to connect to Redis — disabling platform features");
            return None;
        }
    };

    Some(PlatformContext { config, db, redis, jwt_secret })
}
