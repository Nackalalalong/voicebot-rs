use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Tenant {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub plan: String,
    pub is_active: bool,
    pub max_concurrent_sessions: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct User {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub display_name: String,
    pub role: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Campaign {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub status: String,
    pub system_prompt: String,
    pub language: String,
    pub voice_id: Option<String>,
    pub asr_provider: String,
    pub tts_provider: String,
    pub llm_provider: String,
    pub llm_model: String,
    pub max_call_duration_secs: i32,
    pub recording_enabled: bool,
    pub tools_config: serde_json::Value,
    pub custom_metrics: serde_json::Value,
    pub schedule_config: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PromptVersion {
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub tenant_id: Uuid,
    pub version: i32,
    pub system_prompt: String,
    pub change_note: Option<String>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Contact {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub campaign_id: Uuid,
    pub phone_number: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub metadata: serde_json::Value,
    pub status: String,
    pub retry_count: i32,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CallRecord {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub campaign_id: Option<Uuid>,
    pub contact_id: Option<Uuid>,
    pub session_id: String,
    pub direction: String,
    pub phone_number: String,
    pub status: String,
    pub duration_secs: Option<i32>,
    pub recording_url: Option<String>,
    pub transcript: Option<serde_json::Value>,
    pub sentiment: Option<String>,
    pub custom_metrics: serde_json::Value,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PhoneNumber {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub number: String,
    pub provider: String,
    pub provider_number_id: Option<String>,
    pub status: String,
    pub capabilities: serde_json::Value,
    pub monthly_cost_usd_cents: Option<i64>,
    pub campaign_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct UsageRecord {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub campaign_id: Option<Uuid>,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub call_count: i64,
    pub total_duration_secs: i64,
    pub asr_seconds: i64,
    pub tts_characters: i64,
    pub llm_tokens: i64,
    pub cost_usd_cents: i64,
    pub created_at: DateTime<Utc>,
}
