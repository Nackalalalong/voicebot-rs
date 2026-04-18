use std::env;

use apalis::prelude::{Monitor, WorkerBuilder, WorkerBuilderExt, WorkerFactoryFn};
use apalis_sql::postgres::{PgPool, PostgresStorage};
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

mod dialer;
mod jobs;
mod post_call;

use dialer::handle_outbound_call;
use jobs::{OutboundCallJob, PostCallAnalysisJob};
use post_call::handle_post_call_analysis;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(fmt::layer().json())
        .with(EnvFilter::from_default_env())
        .init();

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    let pool = PgPool::connect(&database_url).await?;

    // Run apalis schema migrations (creates the jobs table if not present).
    // Called once — the function is not generic; it creates a shared table.
    PostgresStorage::setup(&pool).await?;

    let outbound_storage: PostgresStorage<OutboundCallJob> =
        PostgresStorage::new(pool.clone());
    let analysis_storage: PostgresStorage<PostCallAnalysisJob> =
        PostgresStorage::new(pool.clone());

    info!("voicebot-scheduler starting");

    Monitor::new()
        .register({
            WorkerBuilder::new("outbound-dialer")
                .concurrency(4)
                .backend(outbound_storage)
                .build_fn(handle_outbound_call)
        })
        .register({
            WorkerBuilder::new("post-call-analysis")
                .concurrency(2)
                .backend(analysis_storage)
                .build_fn(handle_post_call_analysis)
        })
        .run()
        .await?;

    info!("voicebot-scheduler shut down");
    Ok(())
}
