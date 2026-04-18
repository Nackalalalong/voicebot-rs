use std::time::Instant;
use uuid::Uuid;

/// Accumulated statistics for a completed pipeline session.
/// Returned by [`crate::session::PipelineSession::terminate`].
#[derive(Debug, Clone)]
pub struct SessionStats {
    pub session_id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub campaign_id: Option<Uuid>,
    pub started_at: Instant,
    pub ended_at: Instant,
    pub turn_count: u32,
    pub interrupt_count: u32,
}

impl SessionStats {
    pub fn duration_secs(&self) -> f32 {
        self.ended_at.duration_since(self.started_at).as_secs_f32()
    }
}
