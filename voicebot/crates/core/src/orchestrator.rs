use std::sync::Arc;

use agent::core::AgentCore;
use common::events::PipelineEvent;
use common::traits::{LlmProvider, TtsProvider};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchestratorState {
    Idle,
    Listening,
    Transcribing,
    AgentThinking,
    Speaking,
}

pub struct Orchestrator {
    state: OrchestratorState,
    session_id: Uuid,
    event_rx: Receiver<PipelineEvent>,
    event_tx: Option<Sender<PipelineEvent>>,
    egress_tx: Sender<PipelineEvent>,
    cancel_token: CancellationToken,

    // Optional providers — when set, orchestrator triggers downstream components
    llm: Option<Arc<dyn LlmProvider>>,
    tts: Option<Arc<dyn TtsProvider>>,

    // Active task handles for cancellation
    agent_handle: Option<JoinHandle<()>>,
    tts_handle: Option<JoinHandle<()>>,
}

impl Orchestrator {
    /// Create an orchestrator without providers (state machine only, for unit tests).
    pub fn new(
        session_id: Uuid,
        event_rx: Receiver<PipelineEvent>,
        egress_tx: Sender<PipelineEvent>,
        cancel_token: CancellationToken,
    ) -> Self {
        Self {
            state: OrchestratorState::Idle,
            session_id,
            event_rx,
            event_tx: None,
            egress_tx,
            cancel_token,
            llm: None,
            tts: None,
            agent_handle: None,
            tts_handle: None,
        }
    }

    /// Create an orchestrator with providers for full pipeline triggering.
    pub fn with_providers(
        session_id: Uuid,
        event_rx: Receiver<PipelineEvent>,
        event_tx: Sender<PipelineEvent>,
        egress_tx: Sender<PipelineEvent>,
        cancel_token: CancellationToken,
        llm: Arc<dyn LlmProvider>,
        tts: Arc<dyn TtsProvider>,
    ) -> Self {
        Self {
            state: OrchestratorState::Idle,
            session_id,
            event_rx,
            event_tx: Some(event_tx),
            egress_tx,
            cancel_token,
            llm: Some(llm),
            tts: Some(tts),
            agent_handle: None,
            tts_handle: None,
        }
    }

    pub fn state(&self) -> OrchestratorState {
        self.state
    }

    pub async fn run(&mut self) {
        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    info!(session_id = %self.session_id, "orchestrator cancelled");
                    self.state = OrchestratorState::Idle;
                    break;
                }
                event = self.event_rx.recv() => {
                    match event {
                        Some(ev) => self.handle_event(ev).await,
                        None => {
                            debug!(session_id = %self.session_id, "event channel closed, stopping orchestrator");
                            break;
                        }
                    }
                }
            }
        }
    }

    async fn handle_event(&mut self, event: PipelineEvent) {
        match (&self.state, &event) {
            // Valid transitions
            (OrchestratorState::Idle, PipelineEvent::SpeechStarted { .. }) => {
                self.state = OrchestratorState::Listening;
            }
            (OrchestratorState::Listening, PipelineEvent::SpeechEnded { .. }) => {
                self.state = OrchestratorState::Transcribing;
            }

            // Forward partial transcripts while transcribing
            (OrchestratorState::Transcribing, PipelineEvent::PartialTranscript { .. }) => {
                let _ = self.egress_tx.send(event).await;
                return;
            }
            (OrchestratorState::Transcribing, PipelineEvent::FinalTranscript { ref text, .. }) => {
                let transcript = text.clone();
                self.state = OrchestratorState::AgentThinking;
                let _ = self.egress_tx.send(event).await;
                self.trigger_agent(transcript);
                return;
            }

            // Forward agent partial responses
            (OrchestratorState::AgentThinking, PipelineEvent::AgentPartialResponse { .. }) => {
                let _ = self.egress_tx.send(event).await;
                return;
            }
            (
                OrchestratorState::AgentThinking,
                PipelineEvent::AgentFinalResponse { ref text, .. },
            ) => {
                let response_text = text.clone();
                self.state = OrchestratorState::Speaking;
                let _ = self.egress_tx.send(event).await;
                self.trigger_tts(response_text);
                return;
            }

            // Forward TTS audio while speaking
            (OrchestratorState::Speaking, PipelineEvent::TtsAudioChunk { .. }) => {
                let _ = self.egress_tx.send(event).await;
                return;
            }
            (OrchestratorState::Speaking, PipelineEvent::TtsComplete) => {
                self.state = OrchestratorState::Idle;
                let _ = self.egress_tx.send(event).await;
                return;
            }

            // Interrupt — only valid during Speaking
            (OrchestratorState::Speaking, PipelineEvent::Interrupt) => {
                info!(session_id = %self.session_id, "interrupt during speaking, returning to Idle");
                self.cancel_active_tasks();
                crate::observability::record_interrupt();
                self.state = OrchestratorState::Idle;
                return;
            }

            // Cancel — valid from any state
            (_, PipelineEvent::Cancel) => {
                info!(session_id = %self.session_id, state = ?self.state, "cancel received, returning to Idle");
                self.cancel_active_tasks();
                self.state = OrchestratorState::Idle;
                return;
            }

            // Forward component errors from any state
            (
                _,
                PipelineEvent::ComponentError {
                    ref component,
                    ref error,
                    recoverable,
                },
            ) => {
                crate::observability::record_error(&component.to_string(), *recoverable);
                warn!(
                    session_id = %self.session_id,
                    %component,
                    %error,
                    recoverable,
                    "component error"
                );
                let _ = self.egress_tx.send(event).await;
                return;
            }

            // Ignore invalid transitions
            (state, event) => {
                debug!(
                    session_id = %self.session_id,
                    ?state,
                    ?event,
                    "ignoring event in current state"
                );
                return;
            }
        }
    }

    /// Spawn an agent task to handle the transcript (only if LLM provider is configured).
    fn trigger_agent(&mut self, transcript: String) {
        let (Some(llm), Some(event_tx)) = (self.llm.clone(), self.event_tx.clone()) else {
            return;
        };
        let cancel_token = self.cancel_token.child_token();
        let session_id = self.session_id;

        self.agent_handle = Some(tokio::spawn(async move {
            let mut agent = AgentCore::new(llm, vec![], None, cancel_token);
            if let Err(e) = agent.handle_turn(transcript, event_tx).await {
                warn!(session_id = %session_id, "agent error: {}", e);
            }
        }));
    }

    /// Spawn a TTS task to synthesize the response (only if TTS provider is configured).
    fn trigger_tts(&mut self, text: String) {
        let (Some(tts), Some(event_tx)) = (self.tts.clone(), self.event_tx.clone()) else {
            return;
        };
        let session_id = self.session_id;
        let (text_tx, text_rx) = tokio::sync::mpsc::channel::<String>(20);

        self.tts_handle = Some(tokio::spawn(async move {
            // Send text to TTS provider, then drop sender to signal end
            if text_tx.send(text).await.is_err() {
                warn!(session_id = %session_id, "TTS text channel closed");
                return;
            }
            drop(text_tx);
            if let Err(e) = tts.synthesize(text_rx, event_tx).await {
                warn!(session_id = %session_id, "TTS error: {}", e);
            }
        }));
    }

    /// Cancel any active agent or TTS tasks.
    fn cancel_active_tasks(&mut self) {
        if let Some(handle) = self.agent_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.tts_handle.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    async fn create_orchestrator() -> (
        Orchestrator,
        Sender<PipelineEvent>,
        Receiver<PipelineEvent>,
        CancellationToken,
    ) {
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(200);
        let (egress_tx, egress_rx) = tokio::sync::mpsc::channel(200);
        let cancel_token = CancellationToken::new();
        let session_id = Uuid::new_v4();
        let orch = Orchestrator::new(session_id, event_rx, egress_tx, cancel_token.clone());
        (orch, event_tx, egress_rx, cancel_token)
    }

    #[tokio::test]
    async fn test_orchestrator_idle_to_listening() {
        let (mut orch, event_tx, _egress_rx, cancel_token) = create_orchestrator().await;
        assert_eq!(orch.state(), OrchestratorState::Idle);

        event_tx
            .send(PipelineEvent::SpeechStarted { timestamp_ms: 0 })
            .await
            .expect("send failed");
        drop(event_tx);

        timeout(Duration::from_secs(2), orch.run())
            .await
            .expect("orchestrator timed out");

        assert_eq!(orch.state(), OrchestratorState::Listening);
        cancel_token.cancel();
    }

    #[tokio::test]
    async fn test_orchestrator_full_cycle() {
        let (mut orch, event_tx, _egress_rx, cancel_token) = create_orchestrator().await;

        event_tx
            .send(PipelineEvent::SpeechStarted { timestamp_ms: 0 })
            .await
            .expect("send failed");
        event_tx
            .send(PipelineEvent::SpeechEnded { timestamp_ms: 500 })
            .await
            .expect("send failed");
        event_tx
            .send(PipelineEvent::FinalTranscript {
                text: "hello".into(),
                language: "en".into(),
            })
            .await
            .expect("send failed");
        event_tx
            .send(PipelineEvent::AgentFinalResponse {
                text: "hi there".into(),
                tool_calls: vec![],
            })
            .await
            .expect("send failed");
        event_tx
            .send(PipelineEvent::TtsComplete)
            .await
            .expect("send failed");
        drop(event_tx);

        timeout(Duration::from_secs(2), orch.run())
            .await
            .expect("orchestrator timed out");

        assert_eq!(orch.state(), OrchestratorState::Idle);
        cancel_token.cancel();
    }

    #[tokio::test]
    async fn test_orchestrator_interrupt_during_speaking() {
        let (mut orch, event_tx, _egress_rx, cancel_token) = create_orchestrator().await;

        // Drive to Speaking state
        event_tx
            .send(PipelineEvent::SpeechStarted { timestamp_ms: 0 })
            .await
            .expect("send failed");
        event_tx
            .send(PipelineEvent::SpeechEnded { timestamp_ms: 500 })
            .await
            .expect("send failed");
        event_tx
            .send(PipelineEvent::FinalTranscript {
                text: "hello".into(),
                language: "en".into(),
            })
            .await
            .expect("send failed");
        event_tx
            .send(PipelineEvent::AgentFinalResponse {
                text: "hi".into(),
                tool_calls: vec![],
            })
            .await
            .expect("send failed");
        event_tx
            .send(PipelineEvent::Interrupt)
            .await
            .expect("send failed");
        drop(event_tx);

        timeout(Duration::from_secs(2), orch.run())
            .await
            .expect("orchestrator timed out");

        assert_eq!(orch.state(), OrchestratorState::Idle);
        cancel_token.cancel();
    }

    #[tokio::test]
    async fn test_orchestrator_cancel_from_any_state() {
        let (mut orch, event_tx, _egress_rx, cancel_token) = create_orchestrator().await;

        // Drive to AgentThinking state
        event_tx
            .send(PipelineEvent::SpeechStarted { timestamp_ms: 0 })
            .await
            .expect("send failed");
        event_tx
            .send(PipelineEvent::SpeechEnded { timestamp_ms: 500 })
            .await
            .expect("send failed");
        event_tx
            .send(PipelineEvent::FinalTranscript {
                text: "hello".into(),
                language: "en".into(),
            })
            .await
            .expect("send failed");
        event_tx
            .send(PipelineEvent::Cancel)
            .await
            .expect("send failed");
        drop(event_tx);

        timeout(Duration::from_secs(2), orch.run())
            .await
            .expect("orchestrator timed out");

        assert_eq!(orch.state(), OrchestratorState::Idle);
        cancel_token.cancel();
    }
}
