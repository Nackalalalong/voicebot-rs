use apalis::prelude::Error;
use tracing::info;

use crate::jobs::PostCallAnalysisJob;

/// Handler invoked by the apalis worker for each post-call analysis job.
pub async fn handle_post_call_analysis(job: PostCallAnalysisJob) -> Result<(), Error> {
    info!(
        session_id = %job.session_id,
        call_record_id = %job.call_record_id,
        "running post-call analysis"
    );

    // TODO: Integrate with LLM provider to:
    // 1. Analyse transcript sentiment
    // 2. Extract custom metrics defined in campaign config
    // 3. Update call_records.sentiment and call_records.custom_metrics
    // 4. Flush usage stats to usage_records table

    Ok(())
}
