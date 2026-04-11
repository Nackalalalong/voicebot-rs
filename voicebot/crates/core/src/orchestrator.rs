use common::events::PipelineEvent;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};
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
    egress_tx: Sender<PipelineEvent>,
    cancel_token: CancellationToken,
}

impl Orchestrator {
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
            egress_tx,
            cancel_token,
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
            (OrchestratorState::Transcribing, PipelineEvent::FinalTranscript { .. }) => {
                self.state = OrchestratorState::AgentThinking;
                let _ = self.egress_tx.send(event).await;
                return;
            }

            // Forward agent partial responses
            (OrchestratorState::AgentThinking, PipelineEvent::AgentPartialResponse { .. }) => {
                let _ = self.egress_tx.send(event).await;
                return;
            }
            (OrchestratorState::AgentThinking, PipelineEvent::AgentFinalResponse { .. }) => {
                self.state = OrchestratorState::Speaking;
                let _ = self.egress_tx.send(event).await;
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
                self.state = OrchestratorState::Idle;
                return;
            }

            // Cancel — valid from any state
            (_, PipelineEvent::Cancel) => {
                info!(session_id = %self.session_id, state = ?self.state, "cancel received, returning to Idle");
                self.state = OrchestratorState::Idle;
                return;
            }

            // Forward component errors from any state
            (_, PipelineEvent::ComponentError { .. }) => {
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
