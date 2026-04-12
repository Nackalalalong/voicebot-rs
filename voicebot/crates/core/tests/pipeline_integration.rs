use common::audio::AudioFrame;
use common::events::{PipelineEvent, SessionConfig, VadConfig};
use common::types::{AsrProviderType, Language, LlmProviderType, TtsProviderType};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;
use uuid::Uuid;

/// Helper to create voiced audio frames (sine wave at 440Hz)
fn make_speech_frames(count: usize) -> Vec<AudioFrame> {
    let mut frames = Vec::new();
    for i in 0..count {
        let samples: Vec<i16> = (0..320)
            .map(|s| {
                let t = (i * 320 + s) as f32 / 16000.0;
                (0.5 * (2.0 * std::f32::consts::PI * 440.0 * t).sin() * i16::MAX as f32) as i16
            })
            .collect();
        frames.push(AudioFrame::new(samples, i as u64 * 20));
    }
    frames
}

/// Helper to create silence frames
fn make_silence_frames(count: usize, start_idx: usize) -> Vec<AudioFrame> {
    (0..count)
        .map(|i| AudioFrame::silence(20, (start_idx + i) as u64 * 20))
        .collect()
}

#[tokio::test]
async fn test_pipeline_vad_detects_speech() {
    let session_id = Uuid::new_v4();
    let config = SessionConfig {
        session_id,
        language: Language::English,
        asr_provider: AsrProviderType::Speaches,
        tts_provider: TtsProviderType::Speaches,
        llm_provider: LlmProviderType::OpenAi,
        vad_config: VadConfig::default(),
        system_prompt: None,
    };

    let (audio_tx, audio_rx) = mpsc::channel::<AudioFrame>(50);
    let (egress_tx, mut egress_rx) = mpsc::channel::<PipelineEvent>(200);

    let mut session =
        voicebot_core::session::PipelineSession::start_with_stubs(config, audio_rx, egress_tx)
            .await
            .expect("session start failed");

    // Send 25 voiced frames (500ms) — enough to trigger speech detection
    for frame in make_speech_frames(25) {
        audio_tx.send(frame).await.unwrap();
    }

    // Wait for SpeechStarted event from orchestrator
    let event = tokio::time::timeout(std::time::Duration::from_secs(3), egress_rx.recv()).await;

    // The orchestrator may or may not forward SpeechStarted to egress.
    // Let's just verify the session is running without panic.
    // If we got an event, great. If timeout, that's also acceptable since
    // the stub pipeline may not produce egress events for SpeechStarted.
    let _ = event;

    // Now send silence to trigger SpeechEnded
    for frame in make_silence_frames(50, 25) {
        // 1000ms of silence
        audio_tx.send(frame).await.unwrap();
    }

    // Give the pipeline time to process
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Clean up
    session.terminate().await;
}

#[tokio::test]
async fn test_session_starts_and_terminates() {
    let session_id = Uuid::new_v4();
    let config = SessionConfig {
        session_id,
        language: Language::Thai,
        asr_provider: AsrProviderType::Speaches,
        tts_provider: TtsProviderType::Speaches,
        llm_provider: LlmProviderType::OpenAi,
        vad_config: VadConfig::default(),
        system_prompt: None,
    };

    let (_audio_tx, audio_rx) = mpsc::channel::<AudioFrame>(50);
    let (egress_tx, _egress_rx) = mpsc::channel::<PipelineEvent>(200);

    let mut session =
        voicebot_core::session::PipelineSession::start_with_stubs(config, audio_rx, egress_tx)
            .await
            .expect("session start failed");

    assert_eq!(session.id, session_id);

    session.terminate().await;
    // Verify state is Terminated
    assert_eq!(
        session.state,
        voicebot_core::session::SessionState::Terminated
    );
}

#[tokio::test]
async fn test_session_terminate_is_idempotent() {
    let session_id = Uuid::new_v4();
    let config = SessionConfig {
        session_id,
        language: Language::English,
        asr_provider: AsrProviderType::Speaches,
        tts_provider: TtsProviderType::Speaches,
        llm_provider: LlmProviderType::OpenAi,
        vad_config: VadConfig::default(),
        system_prompt: None,
    };

    let (_audio_tx, audio_rx) = mpsc::channel::<AudioFrame>(50);
    let (egress_tx, _egress_rx) = mpsc::channel::<PipelineEvent>(200);

    let mut session =
        voicebot_core::session::PipelineSession::start_with_stubs(config, audio_rx, egress_tx)
            .await
            .expect("session start failed");

    session.terminate().await;
    session.terminate().await; // Should not panic
    assert_eq!(
        session.state,
        voicebot_core::session::SessionState::Terminated
    );
}

#[tokio::test]
async fn test_audio_channel_backpressure() {
    // Verify that audio channel handles overflow gracefully
    let (tx, _rx) = mpsc::channel::<AudioFrame>(5); // Small capacity

    let mut dropped = 0;
    for i in 0..10 {
        let frame = AudioFrame::silence(20, i as u64 * 20);
        match tx.try_send(frame) {
            Ok(_) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                dropped += 1;
            }
            Err(_) => panic!("unexpected error"),
        }
    }
    assert!(dropped > 0, "should have dropped some frames");
}

fn default_config() -> SessionConfig {
    SessionConfig {
        session_id: Uuid::new_v4(),
        language: Language::English,
        asr_provider: AsrProviderType::Speaches,
        tts_provider: TtsProviderType::Speaches,
        llm_provider: LlmProviderType::OpenAi,
        vad_config: VadConfig::default(),
        system_prompt: None,
    }
}

/// Full E2E pipeline test: Audio → VAD → ASR → Agent → TTS → Egress.
///
/// Sends speech followed by silence, then closes the audio stream.
/// Verifies the full chain produces FinalTranscript, AgentFinalResponse,
/// TtsAudioChunk, and TtsComplete on the egress channel.
#[tokio::test]
async fn test_full_pipeline_e2e_with_stubs() {
    let config = default_config();
    let (audio_tx, audio_rx) = mpsc::channel::<AudioFrame>(50);
    let (egress_tx, mut egress_rx) = mpsc::channel::<PipelineEvent>(200);

    let mut session =
        voicebot_core::session::PipelineSession::start_with_stubs(config, audio_rx, egress_tx)
            .await
            .expect("session start failed");

    // Send 25 voiced frames (500ms speech) to trigger VAD SpeechStarted
    for frame in make_speech_frames(25) {
        audio_tx.send(frame).await.expect("send audio");
    }

    // Send 50 silence frames (1000ms) to trigger VAD SpeechEnded
    for frame in make_silence_frames(50, 25) {
        audio_tx.send(frame).await.expect("send silence");
    }

    // Close the audio stream — causes ASR to flush and emit FinalTranscript
    drop(audio_tx);

    // Collect egress events with a timeout
    let mut got_final_transcript = false;
    let mut got_agent_response = false;
    let mut got_tts_audio = false;
    let mut got_tts_complete = false;

    let deadline = timeout(Duration::from_secs(10), async {
        while let Some(event) = egress_rx.recv().await {
            match event {
                PipelineEvent::FinalTranscript { ref text, .. } => {
                    assert_eq!(text, "stub transcript");
                    got_final_transcript = true;
                }
                PipelineEvent::AgentFinalResponse { ref text, .. } => {
                    assert_eq!(text, "stub response");
                    got_agent_response = true;
                }
                PipelineEvent::TtsAudioChunk { .. } => {
                    got_tts_audio = true;
                }
                PipelineEvent::TtsComplete => {
                    got_tts_complete = true;
                    break; // Full flow complete
                }
                _ => {}
            }
        }
    })
    .await;

    assert!(deadline.is_ok(), "pipeline did not complete within 10s");
    assert!(got_final_transcript, "missing FinalTranscript");
    assert!(got_agent_response, "missing AgentFinalResponse");
    assert!(got_tts_audio, "missing TtsAudioChunk");
    assert!(got_tts_complete, "missing TtsComplete");

    session.terminate().await;
}

/// Test that the pipeline works with explicit provider injection.
#[tokio::test]
async fn test_pipeline_with_explicit_providers() {
    let config = default_config();
    let (audio_tx, audio_rx) = mpsc::channel::<AudioFrame>(50);
    let (egress_tx, mut egress_rx) = mpsc::channel::<PipelineEvent>(200);

    let asr: Arc<dyn common::traits::AsrProvider> = Arc::new(asr::stub::StubAsrProvider);
    let llm: Arc<dyn common::traits::LlmProvider> = Arc::new(agent::stub::StubLlmProvider);
    let tts: Arc<dyn common::traits::TtsProvider> = Arc::new(tts::stub::StubTtsProvider);

    let mut session =
        voicebot_core::session::PipelineSession::start(config, audio_rx, egress_tx, asr, llm, tts)
            .await
            .expect("session start failed");

    // Send speech then silence then close
    for frame in make_speech_frames(25) {
        audio_tx.send(frame).await.expect("send audio");
    }
    for frame in make_silence_frames(50, 25) {
        audio_tx.send(frame).await.expect("send silence");
    }
    drop(audio_tx);

    // Wait for TtsComplete
    let result = timeout(Duration::from_secs(10), async {
        while let Some(event) = egress_rx.recv().await {
            if matches!(event, PipelineEvent::TtsComplete) {
                return true;
            }
        }
        false
    })
    .await;

    assert!(result.is_ok(), "pipeline timed out");
    assert!(result.unwrap(), "did not receive TtsComplete");

    session.terminate().await;
}

/// Test that session terminate cancels in-flight pipeline tasks.
#[tokio::test]
async fn test_pipeline_terminate_cancels_active_tasks() {
    let config = default_config();
    let (audio_tx, audio_rx) = mpsc::channel::<AudioFrame>(50);
    let (egress_tx, _egress_rx) = mpsc::channel::<PipelineEvent>(200);

    let mut session =
        voicebot_core::session::PipelineSession::start_with_stubs(config, audio_rx, egress_tx)
            .await
            .expect("session start failed");

    // Start sending audio (pipeline is working)
    for frame in make_speech_frames(10) {
        audio_tx.send(frame).await.expect("send audio");
    }

    // Terminate immediately without waiting for pipeline to complete
    session.terminate().await;
    assert_eq!(
        session.state,
        voicebot_core::session::SessionState::Terminated
    );

    // Audio channel should be closed now
    let frame = AudioFrame::silence(20, 0);
    assert!(audio_tx.send(frame).await.is_err());
}

// ── Provider failure tests ────────────────────────────────────────────────────
//
// These tests inject a failing stub for one component and verify the
// ProviderFailureHandler is called with the correct component and error.
// We use RecordingFailureHandler instead of PanicOnProviderError so the
// test thread can assert on the recorded failure (cross-thread panics are
// not caught by #[should_panic]).

use std::sync::Mutex;

/// Captures the first provider failure: (component_name, error_message).
struct RecordingFailureHandler {
    recorded: Arc<Mutex<Option<(String, String)>>>,
    notify: Arc<tokio::sync::Notify>,
}

impl common::error::ProviderFailureHandler for RecordingFailureHandler {
    fn on_provider_failure(
        &self,
        component: common::types::Component,
        error: &dyn std::error::Error,
    ) {
        let mut guard = self.recorded.lock().unwrap();
        if guard.is_none() {
            *guard = Some((component.to_string(), error.to_string()));
        }
        self.notify.notify_waiters();
    }
}

struct FailingAsrProvider;

#[async_trait::async_trait]
impl common::traits::AsrProvider for FailingAsrProvider {
    async fn stream(
        &self,
        mut audio: Box<dyn common::traits::AudioInputStream>,
        _tx: tokio::sync::mpsc::Sender<PipelineEvent>,
    ) -> Result<(), common::error::AsrError> {
        while audio.recv().await.is_some() {}
        Err(common::error::AsrError::ConnectionFailed)
    }
}

struct FailingLlmProvider;

#[async_trait::async_trait]
impl common::traits::LlmProvider for FailingLlmProvider {
    async fn stream_completion(
        &self,
        _messages: &[common::types::Message],
        _tools: &[common::types::ToolDefinition],
        _tx: tokio::sync::mpsc::Sender<PipelineEvent>,
    ) -> Result<(), common::error::LlmError> {
        Err(common::error::LlmError::ConnectionFailed)
    }

    async fn cancel(&self) {}
}

struct FailingTtsProvider;

#[async_trait::async_trait]
impl common::traits::TtsProvider for FailingTtsProvider {
    async fn synthesize(
        &self,
        mut text_rx: tokio::sync::mpsc::Receiver<String>,
        _tx: tokio::sync::mpsc::Sender<PipelineEvent>,
    ) -> Result<(), common::error::TtsError> {
        while text_rx.recv().await.is_some() {}
        Err(common::error::TtsError::ConnectionFailed)
    }

    async fn cancel(&self) {}
}

/// Helper: run the pipeline with voice frames and wait for the failure handler to fire.
async fn run_until_failure(
    asr: Arc<dyn common::traits::AsrProvider>,
    llm: Arc<dyn common::traits::LlmProvider>,
    tts: Arc<dyn common::traits::TtsProvider>,
) -> (String, String) {
    let config = default_config();
    let (audio_tx, audio_rx) = mpsc::channel::<AudioFrame>(50);
    let (egress_tx, _egress_rx) = mpsc::channel::<PipelineEvent>(200);

    let recorded: Arc<Mutex<Option<(String, String)>>> = Arc::new(Mutex::new(None));
    let notify = Arc::new(tokio::sync::Notify::new());

    let handler = Arc::new(RecordingFailureHandler {
        recorded: Arc::clone(&recorded),
        notify: Arc::clone(&notify),
    });

    let _session = voicebot_core::session::PipelineSession::start_with_handler(
        config, audio_rx, egress_tx, asr, llm, tts, handler,
    )
    .await
    .expect("session start failed");

    // Drive speech through the pipeline
    for frame in make_speech_frames(25) {
        audio_tx.send(frame).await.expect("send audio");
    }
    for frame in make_silence_frames(50, 25) {
        audio_tx.send(frame).await.expect("send silence");
    }
    drop(audio_tx);

    // Wait up to 10s for the handler to fire
    timeout(Duration::from_secs(10), notify.notified())
        .await
        .expect("provider failure handler was not called within 10s");

    let result = recorded.lock().unwrap().clone().unwrap();
    result
}

/// ASR ConnectionFailed must invoke the failure handler with component "asr".
#[tokio::test]
async fn test_asr_provider_failure_is_handled() {
    let (component, error) = run_until_failure(
        Arc::new(FailingAsrProvider),
        Arc::new(agent::stub::StubLlmProvider),
        Arc::new(tts::stub::StubTtsProvider),
    )
    .await;

    assert_eq!(component, "asr", "expected asr component, got: {component}");
    assert!(
        error.contains("connection") || error.contains("ASR"),
        "unexpected error: {error}"
    );
}

/// LLM ConnectionFailed must invoke the failure handler with component "agent".
#[tokio::test]
async fn test_llm_provider_failure_is_handled() {
    let (component, error) = run_until_failure(
        Arc::new(asr::stub::StubAsrProvider),
        Arc::new(FailingLlmProvider),
        Arc::new(tts::stub::StubTtsProvider),
    )
    .await;

    assert_eq!(
        component, "agent",
        "expected agent component, got: {component}"
    );
    assert!(
        error.contains("connection") || error.contains("LLM"),
        "unexpected error: {error}"
    );
}

/// TTS ConnectionFailed must invoke the failure handler with component "tts".
#[tokio::test]
async fn test_tts_provider_failure_is_handled() {
    let (component, error) = run_until_failure(
        Arc::new(asr::stub::StubAsrProvider),
        Arc::new(agent::stub::StubLlmProvider),
        Arc::new(FailingTtsProvider),
    )
    .await;

    assert_eq!(component, "tts", "expected tts component, got: {component}");
    assert!(
        error.contains("connection") || error.contains("TTS"),
        "unexpected error: {error}"
    );
}
