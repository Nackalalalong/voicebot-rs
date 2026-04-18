use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Job payload for initiating an outbound call.
/// Stored in the apalis jobs table; no Job trait needed in apalis 0.6.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundCallJob {
    pub tenant_id: Uuid,
    pub campaign_id: Uuid,
    pub contact_id: Uuid,
    pub phone_number: String,
    pub attempt: u32,
}

/// Job payload for post-call analysis (sentiment, custom metrics).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostCallAnalysisJob {
    pub tenant_id: Uuid,
    pub session_id: String,
    pub call_record_id: Uuid,
}
