use serde::{Deserialize, Serialize};

use crate::error::TransportError;

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "session_start")]
    SessionStart {
        language: String,
        asr: String,
        tts: String,
        #[serde(default)]
        sample_rate: Option<u32>,
    },
    #[serde(rename = "session_end")]
    SessionEnd,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    #[serde(rename = "session_ready")]
    SessionReady,
    #[serde(rename = "transcript_partial")]
    TranscriptPartial { text: String },
    #[serde(rename = "transcript_final")]
    TranscriptFinal { text: String },
    #[serde(rename = "agent_partial")]
    AgentPartial { text: String },
    #[serde(rename = "agent_final")]
    AgentFinal { text: String },
    #[serde(rename = "tts_complete")]
    TtsComplete,
    #[serde(rename = "error")]
    Error { code: String, recoverable: bool },
}

pub fn parse_client_message(text: &str) -> Result<ClientMessage, TransportError> {
    serde_json::from_str(text).map_err(|e| TransportError::InvalidJson(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_session_start() {
        let json = r#"{"type": "session_start", "language": "th", "asr": "speaches", "tts": "speaches", "sample_rate": 48000}"#;
        let msg = parse_client_message(json).unwrap();
        assert!(matches!(
            msg,
            ClientMessage::SessionStart {
                sample_rate: Some(48000),
                ..
            }
        ));
    }

    #[test]
    fn test_parse_session_end() {
        let json = r#"{"type": "session_end"}"#;
        let msg = parse_client_message(json).unwrap();
        assert!(matches!(msg, ClientMessage::SessionEnd));
    }

    #[test]
    fn test_parse_invalid_json() {
        let json = r#"{"invalid": true}"#;
        assert!(parse_client_message(json).is_err());
    }

    #[test]
    fn test_serialize_server_messages() {
        let msg = ServerMessage::TranscriptFinal {
            text: "hello".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("transcript_final"));
        assert!(json.contains("hello"));

        let ready = serde_json::to_string(&ServerMessage::SessionReady).unwrap();
        assert!(ready.contains("session_ready"));
    }
}
