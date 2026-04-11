use serde::{Deserialize, Serialize};
use std::fmt;

/// Language for ASR/TTS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Thai,
    English,
    Auto,
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Language::Thai => write!(f, "th"),
            Language::English => write!(f, "en"),
            Language::Auto => write!(f, "auto"),
        }
    }
}

impl Language {
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "th" | "thai" => Language::Thai,
            "en" | "english" => Language::English,
            _ => Language::Auto,
        }
    }
}

/// ASR provider selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AsrProviderType {
    Deepgram,
    Whisper,
    Speaches,
}

impl fmt::Display for AsrProviderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AsrProviderType::Deepgram => write!(f, "deepgram"),
            AsrProviderType::Whisper => write!(f, "whisper"),
            AsrProviderType::Speaches => write!(f, "speaches"),
        }
    }
}

impl AsrProviderType {
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "whisper" => AsrProviderType::Whisper,
            "speaches" => AsrProviderType::Speaches,
            _ => AsrProviderType::Deepgram,
        }
    }
}

/// TTS provider selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TtsProviderType {
    ElevenLabs,
    Coqui,
    Speaches,
}

impl fmt::Display for TtsProviderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TtsProviderType::ElevenLabs => write!(f, "elevenlabs"),
            TtsProviderType::Coqui => write!(f, "coqui"),
            TtsProviderType::Speaches => write!(f, "speaches"),
        }
    }
}

impl TtsProviderType {
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "coqui" => TtsProviderType::Coqui,
            "speaches" => TtsProviderType::Speaches,
            _ => TtsProviderType::ElevenLabs,
        }
    }
}

/// LLM provider selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmProviderType {
    OpenAi,
    Anthropic,
}

impl fmt::Display for LlmProviderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmProviderType::OpenAi => write!(f, "openai"),
            LlmProviderType::Anthropic => write!(f, "anthropic"),
        }
    }
}

impl LlmProviderType {
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "anthropic" => LlmProviderType::Anthropic,
            _ => LlmProviderType::OpenAi,
        }
    }
}

/// Pipeline component identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Component {
    Vad,
    Asr,
    Agent,
    Tts,
    Transport,
    Orchestrator,
}

impl fmt::Display for Component {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Component::Vad => write!(f, "vad"),
            Component::Asr => write!(f, "asr"),
            Component::Agent => write!(f, "agent"),
            Component::Tts => write!(f, "tts"),
            Component::Transport => write!(f, "transport"),
            Component::Orchestrator => write!(f, "orchestrator"),
        }
    }
}

/// Session end reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EndReason {
    ClientDisconnect,
    ServerShutdown,
    Error(String),
    Timeout,
}

/// A tool call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub function: FunctionCall,
}

/// Function call details within a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Tool definition for LLM function calling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Chat message role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// Chat message for LLM conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(text: &str) -> Self {
        Self {
            role: Role::System,
            content: Some(text.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user(text: &str) -> Self {
        Self {
            role: Role::User,
            content: Some(text.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(text: &str) -> Self {
        Self {
            role: Role::Assistant,
            content: Some(text.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant_with_tool_calls(text: &str, calls: &[ToolCall]) -> Self {
        Self {
            role: Role::Assistant,
            content: Some(text.into()),
            tool_calls: Some(calls.to_vec()),
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: &str, result: &str) -> Self {
        Self {
            role: Role::Tool,
            content: Some(result.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_from_str() {
        assert_eq!(Language::from_str_loose("th"), Language::Thai);
        assert_eq!(Language::from_str_loose("en"), Language::English);
        assert_eq!(Language::from_str_loose("xyz"), Language::Auto);
    }

    #[test]
    fn test_message_constructors() {
        let msg = Message::user("hello");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content.as_deref(), Some("hello"));
        assert!(msg.tool_calls.is_none());
    }
}
