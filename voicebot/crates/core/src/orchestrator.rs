use std::sync::Arc;

use agent::core::AgentCore;
use common::error::{PanicOnProviderError, ProviderFailureHandler};
use common::events::PipelineEvent;
use common::traits::{LlmProvider, TtsProvider};
use common::types::Component;
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

    // Error handler for provider failures
    failure_handler: Arc<dyn ProviderFailureHandler>,

    // Active task handles for cancellation
    agent_handle: Option<JoinHandle<()>>,
    tts_handle: Option<JoinHandle<()>>,

    // Sentence-boundary TTS: accumulates partial text and sends complete sentences
    tts_text_tx: Option<Sender<String>>,
    sentence_buffer: String,
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
            tts_text_tx: None,
            sentence_buffer: String::new(),
            failure_handler: Arc::new(PanicOnProviderError),
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
        failure_handler: Arc<dyn ProviderFailureHandler>,
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
            tts_text_tx: None,
            sentence_buffer: String::new(),
            failure_handler,
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
        // Structured debug for every component output (skip raw audio bytes)
        match &event {
            PipelineEvent::SpeechStarted { timestamp_ms } => {
                debug!(session_id = %self.session_id, timestamp_ms, "VAD → SpeechStarted");
            }
            PipelineEvent::SpeechEnded { timestamp_ms } => {
                debug!(session_id = %self.session_id, timestamp_ms, "VAD → SpeechEnded");
            }
            PipelineEvent::PartialTranscript { text, confidence } => {
                debug!(session_id = %self.session_id, %text, confidence, "ASR → PartialTranscript");
            }
            PipelineEvent::FinalTranscript { text, language } => {
                debug!(session_id = %self.session_id, %text, %language, "ASR → FinalTranscript");
            }
            PipelineEvent::TtsAudioChunk { sequence, .. } => {
                debug!(session_id = %self.session_id, sequence, "TTS → TtsAudioChunk");
            }
            PipelineEvent::TtsComplete => {
                debug!(session_id = %self.session_id, "TTS → TtsComplete");
            }
            _ => {}
        }

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

            // Forward agent partial responses — extract sentences for TTS
            (
                OrchestratorState::AgentThinking,
                PipelineEvent::AgentPartialResponse { ref text, .. },
            ) => {
                // Start TTS on first partial if not already started
                if self.tts_text_tx.is_none() {
                    self.start_tts_stream();
                }
                // Accumulate and extract complete sentences
                self.sentence_buffer.push_str(text);
                self.flush_sentences().await;
                let _ = self.egress_tx.send(event).await;
                return;
            }
            (
                OrchestratorState::AgentThinking,
                PipelineEvent::AgentFinalResponse { ref text, .. },
            ) => {
                // Start TTS if we never got partial responses (e.g. non-streaming LLM)
                if self.tts_text_tx.is_none() {
                    self.start_tts_stream();
                }
                // If the final text differs from accumulated partials, use it directly
                if !text.is_empty() && self.sentence_buffer.is_empty() {
                    self.sentence_buffer.push_str(text);
                }
                // Flush any remaining buffered text as the last sentence
                self.flush_remaining().await;
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

            // Barge-in: user starts speaking during TTS playback → interrupt
            (OrchestratorState::Speaking, PipelineEvent::SpeechStarted { .. }) => {
                info!(session_id = %self.session_id, "barge-in during speaking, interrupting TTS");
                self.cancel_active_tasks();
                crate::observability::record_interrupt();
                self.state = OrchestratorState::Listening;
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

        let failure_handler = Arc::clone(&self.failure_handler);
        let session_id = self.session_id;
        self.agent_handle = Some(tokio::spawn(async move {
            info!(session_id = %session_id, transcript = %transcript, "LLM provider call started");
            let mut agent = AgentCore::new(llm, vec![], None, cancel_token);
            let result = agent.handle_turn(transcript, event_tx).await;
            match &result {
                Ok(()) => info!(session_id = %session_id, "LLM provider call completed"),
                Err(e) => warn!(session_id = %session_id, error = %e, "LLM provider call failed"),
            }
            if let Err(e) = result {
                // Cancelled / channel closed means the session is tearing down — not a provider failure.
                if !matches!(
                    e,
                    agent::error::AgentError::Cancelled | agent::error::AgentError::ChannelClosed
                ) {
                    failure_handler.on_provider_failure(Component::Agent, &e);
                }
            }
        }));
    }

    /// Spawn a TTS task to synthesize the response (only if TTS provider is configured).
    /// Returns a Sender for streaming sentences to TTS incrementally.
    fn start_tts_stream(&mut self) {
        let (Some(tts), Some(event_tx)) = (self.tts.clone(), self.event_tx.clone()) else {
            return;
        };
        let (text_tx, text_rx) = tokio::sync::mpsc::channel::<String>(20);

        self.tts_text_tx = Some(text_tx);

        let failure_handler = Arc::clone(&self.failure_handler);
        let session_id = self.session_id;
        self.tts_handle = Some(tokio::spawn(async move {
            info!(session_id = %session_id, "TTS provider call started");
            let result = tts.synthesize(text_rx, event_tx).await;
            match &result {
                Ok(()) => info!(session_id = %session_id, "TTS provider call completed"),
                Err(e) => warn!(session_id = %session_id, error = %e, "TTS provider call failed"),
            }
            if let Err(e) = result {
                // Cancelled / channel closed means the session is tearing down — not a provider failure.
                if !matches!(
                    e,
                    common::error::TtsError::Cancelled | common::error::TtsError::ChannelClosed
                ) {
                    failure_handler.on_provider_failure(Component::Tts, &e);
                }
            }
        }));
    }

    /// Extract complete sentences from the buffer and send each to TTS.
    /// Sentence boundaries: '.', '!', '?', '\n' followed by a space or end of buffer.
    async fn flush_sentences(&mut self) {
        let Some(tx) = &self.tts_text_tx else { return };

        loop {
            // Find the earliest sentence-ending punctuation followed by whitespace
            let boundary = self
                .sentence_buffer
                .char_indices()
                .zip(self.sentence_buffer.chars().skip(1))
                .find(|((_, c), next)| {
                    (*c == '.' || *c == '!' || *c == '?' || *c == '\n') && next.is_whitespace()
                })
                .map(|((i, c), _)| i + c.len_utf8());

            match boundary {
                Some(pos) => {
                    let sentence: String = self.sentence_buffer.drain(..pos).collect();
                    let trimmed = sentence.trim();
                    if !trimmed.is_empty() {
                        debug!(session_id = %self.session_id, sentence = %trimmed, "sending sentence to TTS");
                        if tx.send(trimmed.to_string()).await.is_err() {
                            warn!(session_id = %self.session_id, "TTS text channel closed");
                            self.tts_text_tx = None;
                            return;
                        }
                    }
                }
                None => break,
            }
        }
    }

    /// Flush any remaining text in the buffer (called on AgentFinalResponse).
    async fn flush_remaining(&mut self) {
        let trimmed = self.sentence_buffer.trim().to_string();
        if !trimmed.is_empty() {
            if let Some(tx) = &self.tts_text_tx {
                debug!(session_id = %self.session_id, sentence = %trimmed, "flushing remaining text to TTS");
                if tx.send(trimmed).await.is_err() {
                    warn!(session_id = %self.session_id, "TTS text channel closed");
                }
            }
        }
        self.sentence_buffer.clear();
        // Drop the sender to signal end of text to TTS
        self.tts_text_tx = None;
    }

    /// Cancel any active agent or TTS tasks.
    fn cancel_active_tasks(&mut self) {
        if let Some(handle) = self.agent_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.tts_handle.take() {
            handle.abort();
        }
        self.tts_text_tx = None;
        self.sentence_buffer.clear();
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

    #[tokio::test]
    async fn test_sentence_boundary_extraction() {
        let (mut orch, _event_tx, _egress_rx, _cancel_token) = create_orchestrator().await;
        let (tts_tx, mut tts_rx) = tokio::sync::mpsc::channel::<String>(20);
        orch.tts_text_tx = Some(tts_tx);

        // Simulate partial responses building up text
        orch.sentence_buffer.push_str("Hello world. ");
        orch.flush_sentences().await;

        let sent = tts_rx.try_recv().unwrap();
        assert_eq!(sent, "Hello world.");

        // Partial that doesn't complete a sentence yet
        orch.sentence_buffer.push_str("How are you");
        orch.flush_sentences().await;
        assert!(tts_rx.try_recv().is_err()); // nothing sent yet

        // Complete the sentence
        orch.sentence_buffer.push_str("? I'm fine. ");
        orch.flush_sentences().await;

        let sent1 = tts_rx.try_recv().unwrap();
        assert_eq!(sent1, "How are you?");
        let sent2 = tts_rx.try_recv().unwrap();
        assert_eq!(sent2, "I'm fine.");

        // Flush remaining
        orch.sentence_buffer.push_str("Goodbye");
        orch.flush_remaining().await;
        let sent3 = tts_rx.try_recv().unwrap();
        assert_eq!(sent3, "Goodbye");
    }

    #[tokio::test]
    async fn test_barge_in_during_speaking() {
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
                text: "long response here".into(),
                tool_calls: vec![],
            })
            .await
            .expect("send failed");
        // User barges in while TTS is playing
        event_tx
            .send(PipelineEvent::SpeechStarted { timestamp_ms: 1000 })
            .await
            .expect("send failed");
        drop(event_tx);

        timeout(Duration::from_secs(2), orch.run())
            .await
            .expect("orchestrator timed out");

        // Barge-in should leave us in Listening (ready for new speech)
        assert_eq!(orch.state(), OrchestratorState::Listening);
        cancel_token.cancel();
    }
}
