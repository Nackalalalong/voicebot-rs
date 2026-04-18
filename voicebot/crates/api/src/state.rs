use std::sync::Arc;

use cache::RedisPool;
use db::PgPool;
use storage::StorageClient;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub redis: RedisPool,
    pub storage: StorageClient,
    pub jwt_secret: String,
}

impl AppState {
    pub fn new(db: PgPool, redis: RedisPool, storage: StorageClient, jwt_secret: String) -> Arc<Self> {
        Arc::new(Self { db, redis, storage, jwt_secret })
    }
}
