use common::audio::AudioFrame;
use common::events::{PipelineEvent, SessionConfig};
use common::testing::ReceiverAudioStream;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

use asr::stub::StubAsrProvider;
use common::traits::AsrProvider;
use vad::component::VadComponent;

use crate::error::SessionError;
use crate::orchestrator::Orchestrator;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Starting,
    Active,
    Terminating,
    Terminated,
}

pub struct PipelineSession {
    pub id: Uuid,
    pub state: SessionState,
    cancel_token: CancellationToken,
    task_handles: Vec<tokio::task::JoinHandle<()>>,
}

impl PipelineSession {
    pub async fn start(
        config: SessionConfig,
        audio_rx: Receiver<AudioFrame>,
        egress_tx: Sender<PipelineEvent>,
    ) -> Result<Self, SessionError> {
        let cancel_token = CancellationToken::new();
        let session_id = config.session_id;

        // Event bus — all components send events here, orchestrator consumes
        let (event_tx, event_rx) = tokio::sync::mpsc::channel::<PipelineEvent>(200);

        let mut handles = Vec::new();

        // VAD component: reads audio_rx, emits SpeechStarted/SpeechEnded to event_tx
        let vad_token = cancel_token.child_token();
        let vad_event_tx = event_tx.clone();
        let mut vad = VadComponent::new(config.vad_config.clone(), vad_event_tx, vad_token);
        handles.push(tokio::spawn(async move {
            let audio_stream = ReceiverAudioStream::new(audio_rx);
            vad.run(Box::new(audio_stream)).await;
        }));

        // Stub ASR: reads audio from a channel (not wired to real audio here),
        // emits FinalTranscript to event_tx
        let asr_event_tx = event_tx.clone();
        let (_asr_audio_tx, asr_audio_rx) = tokio::sync::mpsc::channel::<AudioFrame>(100);
        let asr_provider = StubAsrProvider;
        handles.push(tokio::spawn(async move {
            let audio_stream = ReceiverAudioStream::new(asr_audio_rx);
            if let Err(e) = asr_provider.stream(Box::new(audio_stream), asr_event_tx).await {
                warn!("ASR task error: {}", e);
            }
        }));

        // Orchestrator: consumes event_rx, forwards relevant events to egress_tx
        let orch_token = cancel_token.child_token();
        let mut orchestrator = Orchestrator::new(session_id, event_rx, egress_tx, orch_token);
        handles.push(tokio::spawn(async move {
            orchestrator.run().await;
        }));

        info!(session_id = %session_id, "pipeline session started");

        Ok(Self {
            id: session_id,
            state: SessionState::Active,
            cancel_token,
            task_handles: handles,
        })
    }

    pub async fn terminate(&mut self) {
        if self.state == SessionState::Terminated {
            return;
        }
        self.state = SessionState::Terminating;

        // Signal all components to stop
        self.cancel_token.cancel();

        // Join all handles with 5s timeout
        let deadline = tokio::time::Duration::from_secs(5);
        for handle in self.task_handles.drain(..) {
            if tokio::time::timeout(deadline, handle).await.is_err() {
                warn!(session_id = %self.id, "task did not complete within 5s timeout");
            }
        }

        self.state = SessionState::Terminated;
        info!(session_id = %self.id, "pipeline session terminated");
    }
}
