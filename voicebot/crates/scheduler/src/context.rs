/// Shared runtime context injected into every worker handler.
#[derive(Clone)]
pub struct SchedulerContext {
    pub db: db::PgPool,
    pub redis: cache::RedisPool,
    pub http: reqwest::Client,
    pub ari: AriConfig,
    pub llm: LlmConfig,
}

/// Asterisk ARI connection config (read from env at startup).
#[derive(Clone)]
pub struct AriConfig {
    pub base_url: String,   // e.g. "http://asterisk:8088"
    pub username: String,
    pub password: String,
    pub stasis_app: String, // e.g. "voicebot"
}

/// LLM config for post-call analysis.
#[derive(Clone)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl SchedulerContext {
    /// Build context from environment variables. Panics on missing required vars.
    pub fn from_env(
        db: db::PgPool,
        redis: cache::RedisPool,
        http: reqwest::Client,
    ) -> Self {
        let ari = AriConfig {
            base_url: std::env::var("ARI_BASE_URL").unwrap_or_else(|_| "http://localhost:8088".into()),
            username: std::env::var("ARI_USERNAME").unwrap_or_else(|_| "admin".into()),
            password: std::env::var("ARI_PASSWORD").unwrap_or_else(|_| "admin".into()),
            stasis_app: std::env::var("ARI_STASIS_APP").unwrap_or_else(|_| "voicebot".into()),
        };
        let llm = LlmConfig {
            base_url: std::env::var("LLM_BASE_URL").unwrap_or_else(|_| "http://localhost:8000".into()),
            api_key: std::env::var("LLM_API_KEY").unwrap_or_default(),
            model: std::env::var("LLM_ANALYSIS_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into()),
        };
        Self { db, redis, http, ari, llm }
    }
}
