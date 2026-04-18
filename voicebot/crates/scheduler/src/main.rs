use std::env;

use apalis::prelude::{Monitor, WorkerBuilder, WorkerBuilderExt, WorkerFactoryFn};
use apalis_sql::postgres::{PgPool, PostgresStorage};
use axum::{routing::get, Router};
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

mod context;
mod dialer;
mod jobs;
mod post_call;

use context::SchedulerContext;
use dialer::handle_outbound_call;
use jobs::{OutboundCallJob, PostCallAnalysisJob};
use post_call::handle_post_call_analysis;

async fn healthz() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(fmt::layer().json())
        .with(EnvFilter::from_default_env())
        .init();

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let redis_url = env::var("REDIS_URL").expect("REDIS_URL must be set");

    let pool = PgPool::connect(&database_url).await?;
    let redis = cache::connect(&redis_url).await?;
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    // Run apalis schema migrations
    PostgresStorage::setup(&pool).await?;

    let ctx = SchedulerContext::from_env(pool.clone(), redis, http);

    let outbound_storage: PostgresStorage<OutboundCallJob> =
        PostgresStorage::new(pool.clone());
    let analysis_storage: PostgresStorage<PostCallAnalysisJob> =
        PostgresStorage::new(pool.clone());

    // Spawn a minimal health-check HTTP server
    tokio::spawn(async {
        let app = Router::new().route("/healthz", get(healthz));
        let listener = tokio::net::TcpListener::bind("0.0.0.0:8081")
            .await
            .expect("bind :8081");
        axum::serve(listener, app).await.ok();
    });

    info!("voicebot-scheduler starting");

    Monitor::new()
        .register({
            WorkerBuilder::new("outbound-dialer")
                .concurrency(4)
                .data(ctx.clone())
                .backend(outbound_storage)
                .build_fn(handle_outbound_call)
        })
        .register({
            WorkerBuilder::new("post-call-analysis")
                .concurrency(2)
                .data(ctx.clone())
                .backend(analysis_storage)
                .build_fn(handle_post_call_analysis)
        })
        .run()
        .await?;

    info!("voicebot-scheduler shut down");
    Ok(())
}
