use async_trait::async_trait;
use common::types::ToolDefinition;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("tool execution failed: {0}")]
    ExecutionFailed(String),
    #[error("tool timed out")]
    Timeout,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, args: serde_json::Value) -> Result<String, ToolError>;
}
