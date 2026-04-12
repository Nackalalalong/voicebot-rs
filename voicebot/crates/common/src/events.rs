use crate::audio::AudioFrame;
use crate::types::{Component, EndReason, ToolCall};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{AsrProviderType, Language, LlmProviderType, TtsProviderType};

/// VAD configuration parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VadConfig {
    /// Milliseconds of silence before SpeechEnded fires (default 800).
    pub silence_ms: u32,
    /// Minimum speech duration to count (default 200).
    pub min_speech_ms: u32,
    /// Energy threshold 0.0–1.0 (default 0.02).
    pub energy_threshold: f32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            silence_ms: 800,
            min_speech_ms: 200,
            energy_threshold: 0.02,
        }
    }
}

/// Per-session configuration.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub session_id: Uuid,
    pub language: Language,
    pub asr_provider: AsrProviderType,
    pub tts_provider: TtsProviderType,
    pub llm_provider: LlmProviderType,
    pub vad_config: VadConfig,
    /// Optional system prompt sent to the LLM at the start of every session.
    pub system_prompt: Option<String>,
}

/// All pipeline events flowing through the system.
#[derive(Debug, Clone)]
pub enum PipelineEvent {
    // Audio
    Audio(AudioFrame),

    // VAD
    SpeechStarted {
        timestamp_ms: u64,
    },
    SpeechEnded {
        timestamp_ms: u64,
    },

    // ASR
    PartialTranscript {
        text: String,
        confidence: f32,
    },
    FinalTranscript {
        text: String,
        language: String,
    },

    // Agent
    AgentPartialResponse {
        text: String,
    },
    AgentFinalResponse {
        text: String,
        tool_calls: Vec<ToolCall>,
    },

    // TTS
    TtsAudioChunk {
        frame: AudioFrame,
        sequence: u32,
    },
    TtsComplete,

    // Control signals
    Interrupt,
    Cancel,
    Flush,
    Replace,

    // Lifecycle
    SessionStart {
        session_id: Uuid,
        config: SessionConfig,
    },
    SessionEnd {
        session_id: Uuid,
        reason: EndReason,
    },

    // Errors
    ComponentError {
        component: Component,
        error: String,
        recoverable: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vad_config_defaults() {
        let config = VadConfig::default();
        assert_eq!(config.silence_ms, 800);
        assert_eq!(config.min_speech_ms, 200);
        assert!((config.energy_threshold - 0.02).abs() < f32::EPSILON);
    }

    #[test]
    fn test_pipeline_event_debug() {
        let event = PipelineEvent::SpeechStarted { timestamp_ms: 42 };
        let debug = format!("{:?}", event);
        assert!(debug.contains("SpeechStarted"));
    }
}
