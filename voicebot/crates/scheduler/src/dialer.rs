use apalis::prelude::Error;
use tracing::info;

use crate::jobs::OutboundCallJob;

/// Handler invoked by the apalis worker for each outbound call job.
pub async fn handle_outbound_call(job: OutboundCallJob) -> Result<(), Error> {
    info!(
        campaign_id = %job.campaign_id,
        contact_id = %job.contact_id,
        attempt = job.attempt,
        "dispatching outbound call"
    );

    // TODO: Integrate with Asterisk ARI to originate call:
    // 1. Call Asterisk ARI originate endpoint
    // 2. Pass campaign_id as channel variable for Stasis app routing
    // 3. Update contact status to 'calling'

    Ok(())
}
