use apalis::prelude::{Data, Error};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::context::SchedulerContext;
use crate::jobs::PostCallAnalysisJob;

fn abort<E: std::error::Error + Send + Sync + 'static>(e: E) -> Error {
    Error::Abort(Arc::new(Box::new(e)))
}

fn failed_str(s: impl Into<String>) -> Error {
    Error::Failed(Arc::new(Box::new(std::io::Error::new(
        std::io::ErrorKind::Other, s.into()
    ))))
}

/// Handler invoked by the apalis worker for each post-call analysis job.
pub async fn handle_post_call_analysis(
    job: PostCallAnalysisJob,
    ctx: Data<SchedulerContext>,
) -> Result<(), Error> {
    info!(
        session_id = %job.session_id,
        call_record_id = %job.call_record_id,
        "running post-call analysis"
    );

    // D7: Fetch call record to get transcript
    let call_record = match db::queries::call_records::get_by_id(&ctx.db, job.tenant_id, job.call_record_id).await {
        Ok(r) => r,
        Err(e) => {
            error!(call_record_id = %job.call_record_id, error = %e, "call record not found");
            return Err(abort(e));
        }
    };

    let transcript_text = extract_transcript_text(&call_record.transcript);
    if transcript_text.is_empty() {
        info!(call_record_id = %job.call_record_id, "no transcript available, skipping analysis");
        return Ok(());
    }

    // D7: Call LLM for sentiment + custom metric extraction
    let analysis = match run_llm_analysis(&ctx, &transcript_text).await {
        Ok(a) => a,
        Err(e) => {
            warn!(call_record_id = %job.call_record_id, error = %e, "LLM analysis failed");
            return Err(failed_str(e));
        }
    };

    // D8: Write results to call_records
    let custom_metrics = serde_json::json!({
        "sentiment_score": analysis.sentiment_score,
        "summary": analysis.summary,
        "key_topics": analysis.key_topics,
    });

    let _ = db::queries::call_records::set_analysis(
        &ctx.db,
        job.tenant_id,
        job.call_record_id,
        &analysis.sentiment,
        custom_metrics,
    )
    .await;

    info!(
        call_record_id = %job.call_record_id,
        sentiment = %analysis.sentiment,
        "post-call analysis complete"
    );

    Ok(())
}

#[derive(Debug)]
struct AnalysisResult {
    sentiment: String,
    sentiment_score: f64,
    summary: String,
    key_topics: Vec<String>,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    response_format: ResponseFormat,
    max_tokens: u32,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: String,
}

#[derive(Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: &'static str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
}

#[derive(Deserialize)]
struct AnalysisJson {
    sentiment: Option<String>,
    sentiment_score: Option<f64>,
    summary: Option<String>,
    key_topics: Option<Vec<String>>,
}

async fn run_llm_analysis(ctx: &SchedulerContext, transcript: &str) -> Result<AnalysisResult, String> {
    let system_prompt = "You are a call center analytics assistant. Analyse the provided call transcript and respond with a JSON object containing:\n- sentiment: one of 'positive', 'neutral', 'negative'\n- sentiment_score: float from -1.0 (very negative) to 1.0 (very positive)\n- summary: 1-2 sentence summary of the call\n- key_topics: array of up to 5 key topics discussed";

    let user_content = format!("Transcript:\n{transcript}");

    let req = ChatRequest {
        model: &ctx.llm.model,
        messages: vec![
            ChatMessage { role: "system", content: system_prompt.to_string() },
            ChatMessage { role: "user", content: user_content },
        ],
        response_format: ResponseFormat { format_type: "json_object" },
        max_tokens: 512,
    };

    let resp = ctx.http
        .post(format!("{}/v1/chat/completions", ctx.llm.base_url))
        .bearer_auth(&ctx.llm.api_key)
        .json(&req)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("LLM HTTP {}", resp.status()));
    }

    let body: ChatResponse = resp.json().await.map_err(|e| e.to_string())?;
    let content = body.choices.into_iter().next()
        .map(|c| c.message.content)
        .unwrap_or_default();

    let parsed: AnalysisJson = serde_json::from_str(&content)
        .map_err(|e| format!("JSON parse: {e}: {content}"))?;

    Ok(AnalysisResult {
        sentiment: parsed.sentiment.unwrap_or_else(|| "neutral".into()),
        sentiment_score: parsed.sentiment_score.unwrap_or(0.0),
        summary: parsed.summary.unwrap_or_default(),
        key_topics: parsed.key_topics.unwrap_or_default(),
    })
}

/// Flatten a transcript JSON value (array of {role, text} objects) to plain text.
fn extract_transcript_text(transcript: &Option<serde_json::Value>) -> String {
    match transcript {
        Some(serde_json::Value::Array(turns)) => {
            turns.iter().filter_map(|t| {
                let role = t.get("role").and_then(|v| v.as_str()).unwrap_or("?");
                let text = t.get("text").and_then(|v| v.as_str())?;
                Some(format!("{role}: {text}"))
            }).collect::<Vec<_>>().join("\n")
        }
        Some(serde_json::Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

