use apalis::prelude::{Data, Error};
use chrono::{Timelike, Utc};
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::context::SchedulerContext;
use crate::jobs::OutboundCallJob;

const MAX_RETRY_ATTEMPTS: u32 = 3;

fn failed<E: std::error::Error + Send + Sync + 'static>(e: E) -> Error {
    Error::Failed(Arc::new(Box::new(e)))
}

fn abort<E: std::error::Error + Send + Sync + 'static>(e: E) -> Error {
    Error::Abort(Arc::new(Box::new(e)))
}

pub async fn handle_outbound_call(
    job: OutboundCallJob,
    ctx: Data<SchedulerContext>,
) -> Result<(), Error> {
    info!(
        campaign_id = %job.campaign_id,
        contact_id = %job.contact_id,
        attempt = job.attempt,
        "dispatching outbound call"
    );

    let campaign = match db::queries::campaigns::get_by_id(&ctx.db, job.tenant_id, job.campaign_id).await {
        Ok(c) => c,
        Err(e) => {
            error!(campaign_id = %job.campaign_id, error = %e, "campaign not found");
            return Err(abort(e));
        }
    };

    if campaign.status != "active" {
        info!(campaign_id = %job.campaign_id, status = %campaign.status, "campaign not active, skipping");
        return Ok(());
    }

    // D4: Schedule enforcement
    if !is_within_schedule(&campaign.schedule_config) {
        info!(campaign_id = %job.campaign_id, "outside schedule window");
        return Err(failed(std::io::Error::new(std::io::ErrorKind::Other, "outside schedule window")));
    }

    // D5: Rate limiting via Redis INCR with 60s TTL
    let rate_key = format!("campaign:{}:dial_rate_1m", job.campaign_id);
    let call_rate_limit = call_rate_from_config(&campaign.schedule_config);
    let mut redis = ctx.redis.clone();
    if let Ok(count) = increment_rate_counter(&mut redis, &rate_key).await {
        if count > call_rate_limit as i64 {
            info!(campaign_id = %job.campaign_id, count, limit = call_rate_limit, "rate limit hit");
            return Err(failed(std::io::Error::new(std::io::ErrorKind::Other, "rate limit exceeded")));
        }
    }

    // D2: Originate call via Asterisk ARI
    let endpoint = format!("PJSIP/{}", job.phone_number.trim_start_matches('+'));
    let app_args = format!("campaign_id={},contact_id={}", job.campaign_id, job.contact_id);

    #[derive(serde::Serialize)]
    struct OriginateRequest<'a> {
        endpoint: &'a str,
        #[serde(rename = "callerId")]
        caller_id: &'a str,
        app: &'a str,
        #[serde(rename = "appArgs")]
        app_args: &'a str,
    }

    let body = OriginateRequest {
        endpoint: &endpoint,
        caller_id: "VoiceBot <+10000000000>",
        app: &ctx.ari.stasis_app,
        app_args: &app_args,
    };

    let response = ctx
        .http
        .post(format!("{}/ari/channels", ctx.ari.base_url))
        .basic_auth(&ctx.ari.username, Some(&ctx.ari.password))
        .json(&body)
        .send()
        .await;

    match response {
        Ok(resp) if resp.status().is_success() => {
            info!(
                campaign_id = %job.campaign_id,
                contact_id = %job.contact_id,
                phone = %job.phone_number,
                "call originated"
            );
            let _ =
                db::queries::contacts::update_status(&ctx.db, job.tenant_id, job.contact_id, "calling").await;
        }
        Ok(resp) => {
            let status = resp.status();
            warn!(campaign_id = %job.campaign_id, %status, "ARI originate failed");
            handle_call_failure(&ctx, &job).await;
            return Err(failed(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("ARI {status}"),
            )));
        }
        Err(e) => {
            error!(campaign_id = %job.campaign_id, error = %e, "HTTP to ARI failed");
            handle_call_failure(&ctx, &job).await;
            return Err(failed(e));
        }
    }

    // D6: Campaign completion check
    if let Ok(remaining) =
        db::queries::contacts::count_active(&ctx.db, job.tenant_id, job.campaign_id).await
    {
        if remaining == 0 {
            info!(campaign_id = %job.campaign_id, "all contacts dialed — completing campaign");
            let _ =
                db::queries::campaigns::update_status(&ctx.db, job.tenant_id, job.campaign_id, "completed")
                    .await;
        }
    }

    Ok(())
}

// D3: Retry with exponential backoff or mark permanently failed.
async fn handle_call_failure(ctx: &SchedulerContext, job: &OutboundCallJob) {
    if job.attempt >= MAX_RETRY_ATTEMPTS {
        warn!(contact_id = %job.contact_id, "max retries reached, marking failed");
        let _ = db::queries::contacts::update_status(&ctx.db, job.tenant_id, job.contact_id, "failed").await;
    } else {
        let backoff_mins = 5u64 * 3u64.pow(job.attempt);
        let next = Utc::now() + chrono::Duration::minutes(backoff_mins as i64);
        let _ = db::queries::contacts::mark_failed_retry(&ctx.db, job.tenant_id, job.contact_id, next).await;
    }
}

fn is_within_schedule(config: &serde_json::Value) -> bool {
    let start = config.get("start_hour").and_then(|v| v.as_u64());
    let end = config.get("end_hour").and_then(|v| v.as_u64());
    match (start, end) {
        (Some(s), Some(e)) => {
            let h = Utc::now().hour() as u64;
            h >= s && h < e
        }
        _ => true,
    }
}

fn call_rate_from_config(config: &serde_json::Value) -> u64 {
    config
        .get("call_rate_per_minute")
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
}

async fn increment_rate_counter(redis: &mut cache::RedisPool, key: &str) -> Result<i64, redis::RedisError> {
    use redis::AsyncCommands;
    let count: i64 = redis.incr(key, 1i64).await?;
    if count == 1 {
        let _: () = redis.expire(key, 60).await?;
    }
    Ok(count)
}
