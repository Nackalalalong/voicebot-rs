use std::env;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(fmt::layer().json())
        .with(EnvFilter::from_default_env())
        .init();

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into());
    let jwt_secret = env::var("JWT_SECRET").expect("JWT_SECRET must be set");
    let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());

    let storage_config = storage::StorageConfig {
        endpoint_url: env::var("S3_ENDPOINT_URL").unwrap_or_else(|_| "http://localhost:9000".into()),
        access_key: env::var("S3_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".into()),
        secret_key: env::var("S3_SECRET_KEY").unwrap_or_else(|_| "minioadmin".into()),
        region: env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
        bucket: env::var("S3_BUCKET").unwrap_or_else(|_| "voicebot".into()),
        force_path_style: true,
    };

    let db = db::connect(&database_url).await?;
    db::run_migrations(&db).await?;

    let redis = cache::connect(&redis_url).await?;
    let storage = storage::StorageClient::new(storage_config).await?;

    let state = api::state::AppState::new(db, redis, storage, jwt_secret);
    let router = api::create_router(state);

    info!("voicebot-api listening on {bind_addr}");
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}
