use std::sync::Arc;

use async_trait::async_trait;
use common::types::ToolDefinition;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::tool::{Tool, ToolError};

/// A tool that captures a single custom metric value and writes it into a
/// shared map. One instance is created per `agent_tool` metric in the
/// campaign's `custom_metrics` JSONB array.
///
/// The shared `captured` map is read by the transport layer after the session
/// ends and merged into the CDR `custom_metrics` field.
pub struct MetricCaptureTool {
    metric_key: String,
    tool_name: String,
    metric_type: String,
    label: String,
    description: Option<String>,
    enum_options: Option<Vec<String>>,
    captured: Arc<Mutex<serde_json::Map<String, Value>>>,
}

impl MetricCaptureTool {
    pub fn new(
        metric_key: String,
        tool_name: String,
        metric_type: String,
        label: String,
        description: Option<String>,
        enum_options: Option<Vec<String>>,
        captured: Arc<Mutex<serde_json::Map<String, Value>>>,
    ) -> Self {
        Self {
            metric_key,
            tool_name,
            metric_type,
            label,
            description,
            enum_options,
            captured,
        }
    }
}

#[async_trait]
impl Tool for MetricCaptureTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn definition(&self) -> ToolDefinition {
        // Build a JSON Schema for the tool's single `value` parameter.
        let value_schema = match self.metric_type.as_str() {
            "boolean" => serde_json::json!({"type": "boolean"}),
            "number" => serde_json::json!({"type": "number"}),
            "enum" => {
                if let Some(opts) = &self.enum_options {
                    serde_json::json!({"type": "string", "enum": opts})
                } else {
                    serde_json::json!({"type": "string"})
                }
            }
            _ => serde_json::json!({"type": "string"}),
        };

        let description = self
            .description
            .clone()
            .unwrap_or_else(|| format!("Record the value for metric: {}", self.label));

        ToolDefinition {
            name: self.tool_name.clone(),
            description,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "value": value_schema
                },
                "required": ["value"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<String, ToolError> {
        let value = args
            .get("value")
            .cloned()
            .unwrap_or(Value::Null);

        let mut map = self.captured.lock().await;
        map.insert(self.metric_key.clone(), value.clone());

        tracing::debug!(
            tool = %self.tool_name,
            key = %self.metric_key,
            ?value,
            "metric captured"
        );

        Ok(format!("Metric '{}' recorded.", self.label))
    }
}

/// Build a set of `MetricCaptureTool`s from a campaign's `custom_metrics` JSONB array.
///
/// Only metrics with `"collection": "agent_tool"` are turned into tools.
/// Returns the tools and the shared capture map (to be read after session end).
pub fn tools_from_metrics(
    custom_metrics: &Value,
) -> (
    Vec<Box<dyn Tool>>,
    Arc<Mutex<serde_json::Map<String, Value>>>,
) {
    let captured = Arc::new(Mutex::new(serde_json::Map::new()));
    let tools = tools_from_metrics_with_capture(custom_metrics, Arc::clone(&captured));

    (tools, captured)
}

pub fn tools_from_metrics_with_capture(
    custom_metrics: &Value,
    captured: Arc<Mutex<serde_json::Map<String, Value>>>,
) -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();

    let metrics = match custom_metrics.as_array() {
        Some(arr) => arr,
        None => return tools,
    };

    for metric in metrics {
        let collection = metric.get("collection").and_then(Value::as_str);
        if collection != Some("agent_tool") {
            continue;
        }

        let key = match metric.get("key").and_then(Value::as_str) {
            Some(k) => k.to_string(),
            None => continue,
        };
        let tool_name = match metric.get("tool_name").and_then(Value::as_str) {
            Some(n) => n.to_string(),
            None => format!("record_{}", key),
        };
        let metric_type = metric
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("text")
            .to_string();
        let label = metric
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or(&key)
            .to_string();
        let description = metric
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string);
        let enum_options = metric
            .get("options")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            });

        tools.push(Box::new(MetricCaptureTool::new(
            key,
            tool_name,
            metric_type,
            label,
            description,
            enum_options,
            Arc::clone(&captured),
        )));
    }

    tools
}
