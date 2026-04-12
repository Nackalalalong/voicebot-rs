use std::sync::Arc;

use common::audio::AudioFrame;
use common::config::AppConfig;
use common::events::{PipelineEvent, SessionConfig};
use common::testing::ReceiverAudioStream;
use common::traits::{AsrProvider, LlmProvider, TtsProvider};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

use agent::stub::StubLlmProvider;
use asr::stub::StubAsrProvider;
use tts::stub::StubTtsProvider;
use vad::component::VadComponent;

use crate::error::SessionError;
use crate::orchestrator::Orchestrator;

/// Build providers from AppConfig + SessionConfig.
pub fn build_providers(
    app_config: &AppConfig,
    session_config: &SessionConfig,
) -> Result<
    (
        Arc<dyn AsrProvider>,
        Arc<dyn LlmProvider>,
        Arc<dyn TtsProvider>,
    ),
    SessionError,
> {
    use common::types::{AsrProviderType, TtsProviderType};

    // --- ASR ---
    let asr: Arc<dyn AsrProvider> = match session_config.asr_provider {
        AsrProviderType::Speaches => {
            let cfg =
                app_config.asr.speaches.as_ref().ok_or_else(|| {
                    SessionError::Internal("[asr.speaches] config missing".into())
                })?;
            let mut provider =
                asr::speaches::SpeachesAsrProvider::new(cfg.base_url.clone(), cfg.model.clone());
            if let Some(key) = &cfg.api_key {
                provider = provider.with_api_key(key.clone());
            }
            if let Some(lang) = &cfg.language {
                provider = provider.with_language(lang.clone());
            }
            Arc::new(provider)
        }
        AsrProviderType::Whisper => {
            // Whisper local not yet implemented — use stub
            Arc::new(StubAsrProvider)
        }
    };

    // --- LLM ---
    let llm: Arc<dyn LlmProvider> = match session_config.llm_provider {
        common::types::LlmProviderType::OpenAi => {
            let cfg = app_config
                .llm
                .openai
                .as_ref()
                .ok_or_else(|| SessionError::Internal("[llm.openai] config missing".into()))?;
            let provider = agent::openai::OpenAiProvider::new(
                cfg.base_url.clone(),
                cfg.api_key.clone(),
                cfg.model.clone(),
            );
            Arc::new(provider)
        }
        common::types::LlmProviderType::Anthropic => {
            // Anthropic not yet implemented — use stub
            Arc::new(StubLlmProvider)
        }
    };

    // --- TTS ---
    let tts: Arc<dyn TtsProvider> = match session_config.tts_provider {
        TtsProviderType::Speaches => {
            let cfg =
                app_config.tts.speaches.as_ref().ok_or_else(|| {
                    SessionError::Internal("[tts.speaches] config missing".into())
                })?;
            let mut provider = tts::speaches::SpeachesTtsProvider::new(
                cfg.base_url.clone(),
                cfg.model.clone(),
                cfg.voice.clone(),
            );
            if let Some(key) = &cfg.api_key {
                provider = provider.with_api_key(key.clone());
            }
            Arc::new(provider)
        }
        TtsProviderType::Coqui => {
            // Coqui not yet implemented — use stub
            Arc::new(StubTtsProvider)
        }
    };

    Ok((asr, llm, tts))
}

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
    /// Start a full pipeline session with the given providers.
    pub async fn start(
        config: SessionConfig,
        audio_rx: Receiver<AudioFrame>,
        egress_tx: Sender<PipelineEvent>,
        asr: Arc<dyn AsrProvider>,
        llm: Arc<dyn LlmProvider>,
        tts: Arc<dyn TtsProvider>,
    ) -> Result<Self, SessionError> {
        let cancel_token = CancellationToken::new();
        let session_id = config.session_id;

        // Event bus — all components send events here, orchestrator consumes
        let (event_tx, event_rx) = tokio::sync::mpsc::channel::<PipelineEvent>(200);

        let mut handles = Vec::new();

        // Audio fanout — fork incoming audio to both VAD and ASR
        let (vad_audio_tx, vad_audio_rx) = tokio::sync::mpsc::channel::<AudioFrame>(50);
        let (asr_audio_tx, asr_audio_rx) = tokio::sync::mpsc::channel::<AudioFrame>(100);
        let fanout_token = cancel_token.child_token();
        handles.push(tokio::spawn(async move {
            let mut rx = audio_rx;
            loop {
                tokio::select! {
                    _ = fanout_token.cancelled() => break,
                    frame = rx.recv() => {
                        match frame {
                            Some(f) => {
                                // Clone is cheap — AudioFrame uses Arc<[i16]>
                                let _ = vad_audio_tx.try_send(f.clone());
                                let _ = asr_audio_tx.try_send(f);
                            }
                            None => break,
                        }
                    }
                }
            }
        }));

        // VAD component: reads audio, emits SpeechStarted/SpeechEnded to event_tx
        let vad_token = cancel_token.child_token();
        let vad_event_tx = event_tx.clone();
        let mut vad = VadComponent::new(config.vad_config.clone(), vad_event_tx, vad_token);
        handles.push(tokio::spawn(async move {
            let audio_stream = ReceiverAudioStream::new(vad_audio_rx);
            vad.run(Box::new(audio_stream)).await;
        }));

        // ASR: reads audio, emits FinalTranscript to event_tx
        let asr_event_tx = event_tx.clone();
        handles.push(tokio::spawn(async move {
            let audio_stream = ReceiverAudioStream::new(asr_audio_rx);
            if let Err(e) = asr.stream(Box::new(audio_stream), asr_event_tx).await {
                warn!("ASR task error: {}", e);
            }
        }));

        // Orchestrator: consumes events, triggers agent/TTS, forwards to egress
        let orch_token = cancel_token.child_token();
        let mut orchestrator = Orchestrator::with_providers(
            session_id,
            event_rx,
            event_tx.clone(),
            egress_tx,
            orch_token,
            llm,
            tts,
        );
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

    /// Start a session with stub providers (for testing and development).
    pub async fn start_with_stubs(
        config: SessionConfig,
        audio_rx: Receiver<AudioFrame>,
        egress_tx: Sender<PipelineEvent>,
    ) -> Result<Self, SessionError> {
        let asr: Arc<dyn AsrProvider> = Arc::new(StubAsrProvider);
        let llm: Arc<dyn LlmProvider> = Arc::new(StubLlmProvider);
        let tts: Arc<dyn TtsProvider> = Arc::new(StubTtsProvider);
        Self::start(config, audio_rx, egress_tx, asr, llm, tts).await
    }

    /// Start a session using providers derived from AppConfig.
    pub async fn start_with_config(
        app_config: &AppConfig,
        session_config: SessionConfig,
        audio_rx: Receiver<AudioFrame>,
        egress_tx: Sender<PipelineEvent>,
    ) -> Result<Self, SessionError> {
        let (asr, llm, tts) = build_providers(app_config, &session_config)?;
        Self::start(session_config, audio_rx, egress_tx, asr, llm, tts).await
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
