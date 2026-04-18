use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use common::audio::AudioFrame;
use common::config::AppConfig;
use common::error::{
    AsrError, ComponentErrorTrait, LlmError, LogOnProviderError, ProviderFailureHandler,
};
use common::events::{PipelineEvent, SessionConfig};
use common::testing::ReceiverAudioStream;
use common::traits::{AsrProvider, AudioInputStream, LlmProvider, TtsProvider};
use common::types::{AsrProviderType, Component, LlmProviderType, TtsProviderType};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

use agent::core::AgentCore;
use agent::stub::StubLlmProvider;
use asr::stub::StubAsrProvider;
use tts::stub::StubTtsProvider;
use vad::component::VadComponent;

use crate::error::SessionError;
use crate::orchestrator::Orchestrator;
use crate::stats::SessionStats;

struct BufferedAudioStream {
    frames: VecDeque<AudioFrame>,
}

struct ActiveAsrTurn {
    turn_id: u64,
    audio_tx: Option<Sender<AudioFrame>>,
    cancel_token: CancellationToken,
    capture_open: bool,
}

impl ActiveAsrTurn {
    fn cancel(&mut self) {
        self.cancel_token.cancel();
        self.audio_tx = None;
        self.capture_open = false;
    }
}

impl BufferedAudioStream {
    fn new(frames: Vec<AudioFrame>) -> Self {
        Self {
            frames: frames.into(),
        }
    }
}

#[async_trait]
impl AudioInputStream for BufferedAudioStream {
    async fn recv(&mut self) -> Option<AudioFrame> {
        self.frames.pop_front()
    }
}

struct FallbackAsrProvider {
    primary: Arc<dyn AsrProvider>,
    fallback: Option<Arc<dyn AsrProvider>>,
}

impl FallbackAsrProvider {
    const MAX_ATTEMPTS: u32 = 3;
    const BASE_DELAY_MS: u64 = 200;

    fn new(primary: Arc<dyn AsrProvider>, fallback: Option<Arc<dyn AsrProvider>>) -> Self {
        Self { primary, fallback }
    }

    async fn stream_with_retries(
        provider: &Arc<dyn AsrProvider>,
        frames: &[AudioFrame],
        tx: &Sender<PipelineEvent>,
        provider_label: &str,
    ) -> Result<(), AsrError> {
        let mut attempt = 1;

        loop {
            let stream = BufferedAudioStream::new(frames.to_vec());
            match provider.stream(Box::new(stream), tx.clone()).await {
                Ok(()) => return Ok(()),
                Err(err) if err.is_recoverable() && attempt < Self::MAX_ATTEMPTS => {
                    let delay_ms = err
                        .retry_after_ms()
                        .unwrap_or(Self::BASE_DELAY_MS * attempt as u64);
                    warn!(
                        attempt,
                        delay_ms,
                        error = %err,
                        provider = provider_label,
                        "ASR attempt failed, retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    attempt += 1;
                }
                Err(err) => return Err(err),
            }
        }
    }
}

#[async_trait]
impl AsrProvider for FallbackAsrProvider {
    async fn stream(
        &self,
        mut audio: Box<dyn AudioInputStream>,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), AsrError> {
        let mut frames = Vec::new();
        while let Some(frame) = audio.recv().await {
            frames.push(frame);
        }

        match Self::stream_with_retries(&self.primary, &frames, &tx, "primary").await {
            Ok(()) => Ok(()),
            Err(err) if err.is_recoverable() => match &self.fallback {
                Some(fallback) => {
                    warn!(error = %err, "ASR primary exhausted retries, switching to fallback");
                    Self::stream_with_retries(fallback, &frames, &tx, "fallback").await
                }
                None => Err(err),
            },
            Err(err) => Err(err),
        }
    }

    async fn cancel(&self) {
        self.primary.cancel().await;
        if let Some(fallback) = &self.fallback {
            fallback.cancel().await;
        }
    }
}

struct FallbackLlmProvider {
    primary: Arc<dyn LlmProvider>,
    fallback: Option<Arc<dyn LlmProvider>>,
}

impl FallbackLlmProvider {
    const MAX_ATTEMPTS: u32 = 2;
    const BASE_DELAY_MS: u64 = 500;

    fn new(primary: Arc<dyn LlmProvider>, fallback: Option<Arc<dyn LlmProvider>>) -> Self {
        Self { primary, fallback }
    }

    async fn stream_with_retries(
        provider: &Arc<dyn LlmProvider>,
        messages: &[common::types::Message],
        tools: &[common::types::ToolDefinition],
        tx: &Sender<PipelineEvent>,
        provider_label: &str,
    ) -> Result<(), LlmError> {
        let mut attempt = 1;

        loop {
            match provider
                .stream_completion(messages, tools, tx.clone())
                .await
            {
                Ok(()) => return Ok(()),
                Err(err) if err.is_recoverable() && attempt < Self::MAX_ATTEMPTS => {
                    let delay_ms = err
                        .retry_after_ms()
                        .unwrap_or(Self::BASE_DELAY_MS * 2u64.pow((attempt - 1) as u32));
                    warn!(
                        attempt,
                        delay_ms,
                        error = %err,
                        provider = provider_label,
                        "LLM attempt failed, retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    attempt += 1;
                }
                Err(err) => return Err(err),
            }
        }
    }
}

#[async_trait]
impl LlmProvider for FallbackLlmProvider {
    async fn stream_completion(
        &self,
        messages: &[common::types::Message],
        tools: &[common::types::ToolDefinition],
        tx: Sender<PipelineEvent>,
    ) -> Result<(), LlmError> {
        match Self::stream_with_retries(&self.primary, messages, tools, &tx, "primary").await {
            Ok(()) => Ok(()),
            Err(err) if err.is_recoverable() => match &self.fallback {
                Some(fallback) => {
                    warn!(error = %err, "LLM primary exhausted retries, switching to fallback");
                    Self::stream_with_retries(fallback, messages, tools, &tx, "fallback").await
                }
                None => Err(err),
            },
            Err(err) => Err(err),
        }
    }

    async fn cancel(&self) {
        self.primary.cancel().await;
        if let Some(fallback) = &self.fallback {
            fallback.cancel().await;
        }
    }
}

fn build_asr_provider_for_type(
    app_config: &AppConfig,
    provider_type: AsrProviderType,
) -> Result<Arc<dyn AsrProvider>, SessionError> {
    match provider_type {
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
            Ok(Arc::new(provider))
        }
        AsrProviderType::Whisper => Ok(Arc::new(StubAsrProvider)),
    }
}

fn build_llm_provider_for_type(
    app_config: &AppConfig,
    provider_type: LlmProviderType,
) -> Result<Arc<dyn LlmProvider>, SessionError> {
    match provider_type {
        LlmProviderType::OpenAi => {
            let cfg = app_config
                .llm
                .openai
                .as_ref()
                .ok_or_else(|| SessionError::Internal("[llm.openai] config missing".into()))?;
            Ok(Arc::new(agent::openai::OpenAiProvider::new(
                cfg.base_url.clone(),
                cfg.api_key.clone(),
                cfg.model.clone(),
            )))
        }
        LlmProviderType::Anthropic => Ok(Arc::new(StubLlmProvider)),
    }
}

fn build_tts_provider_for_type(
    app_config: &AppConfig,
    provider_type: TtsProviderType,
) -> Result<Arc<dyn TtsProvider>, SessionError> {
    match provider_type {
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
            Ok(Arc::new(provider))
        }
        TtsProviderType::Coqui => Ok(Arc::new(StubTtsProvider)),
    }
}

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
    let configured_asr_primary = AsrProviderType::from_str_loose(&app_config.asr.primary);
    let effective_asr_primary = if session_config.asr_provider == configured_asr_primary {
        configured_asr_primary
    } else {
        session_config.asr_provider.clone()
    };
    let asr_fallback_type = app_config
        .asr
        .fallback
        .as_deref()
        .map(AsrProviderType::from_str_loose)
        .filter(|provider| *provider != effective_asr_primary);
    let asr_primary = build_asr_provider_for_type(app_config, effective_asr_primary.clone())?;
    let asr_fallback = match asr_fallback_type {
        Some(fallback_type) => Some(build_asr_provider_for_type(app_config, fallback_type)?),
        None => None,
    };
    // Always wrap ASR so transient recoverable errors get bounded retries,
    // even when no distinct fallback provider is configured.
    let asr: Arc<dyn AsrProvider> = Arc::new(FallbackAsrProvider::new(asr_primary, asr_fallback));

    let configured_llm_primary = LlmProviderType::from_str_loose(&app_config.llm.primary);
    let effective_llm_primary = if session_config.llm_provider == configured_llm_primary {
        configured_llm_primary
    } else {
        session_config.llm_provider.clone()
    };
    let llm_fallback_type = app_config
        .llm
        .fallback
        .as_deref()
        .map(LlmProviderType::from_str_loose)
        .filter(|provider| *provider != effective_llm_primary);
    let llm_primary = build_llm_provider_for_type(app_config, effective_llm_primary.clone())?;
    let llm: Arc<dyn LlmProvider> = if let Some(fallback_type) = llm_fallback_type {
        let fallback = build_llm_provider_for_type(app_config, fallback_type)?;
        Arc::new(FallbackLlmProvider::new(llm_primary, Some(fallback)))
    } else {
        llm_primary
    };

    let tts = build_tts_provider_for_type(app_config, session_config.tts_provider.clone())?;

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
    pub tenant_id: Option<Uuid>,
    pub campaign_id: Option<Uuid>,
    started_at: Instant,
    cancel_token: CancellationToken,
    task_handles: Vec<tokio::task::JoinHandle<()>>,
    // Shared with the Orchestrator task so we can read them after it exits.
    turn_count: Arc<AtomicU32>,
    interrupt_count: Arc<AtomicU32>,
    agent_controller: Option<SessionAgentController>,
}

#[derive(Clone)]
pub struct SessionAgentController {
    agent: Arc<Mutex<AgentCore>>,
}

impl SessionAgentController {
    pub async fn reload_agent_config(
        &self,
        system_prompt: Option<String>,
        tools: Vec<Box<dyn agent::tool::Tool>>,
    ) {
        let mut agent = self.agent.lock().await;
        agent.reload_runtime_config(system_prompt, tools).await;
    }
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
        Self::start_with_handler(
            config,
            audio_rx,
            egress_tx,
            asr,
            llm,
            tts,
            Arc::new(LogOnProviderError),
        )
        .await
    }

    /// Start with an explicit failure handler (useful for testing).
    pub async fn start_with_handler(
        config: SessionConfig,
        audio_rx: Receiver<AudioFrame>,
        egress_tx: Sender<PipelineEvent>,
        asr: Arc<dyn AsrProvider>,
        llm: Arc<dyn LlmProvider>,
        tts: Arc<dyn TtsProvider>,
        failure_handler: Arc<dyn ProviderFailureHandler>,
    ) -> Result<Self, SessionError> {
        let cancel_token = CancellationToken::new();
        let session_id = config.session_id;

        // Event bus — all components send events here, orchestrator consumes
        let (event_tx, event_rx) = tokio::sync::mpsc::channel::<PipelineEvent>(200);

        let mut handles = Vec::new();

        // Audio fanout — gateway for VAD audio forwarding
        // VAD receives all frames continuously; ASR receives only speech utterances.
        // Small capacity forces send().await to yield when VAD lags, giving it
        // CPU time to process frames and fire speech events into speech_state_rx.
        let (vad_audio_tx, vad_audio_rx) = tokio::sync::mpsc::channel::<AudioFrame>(16);

        // Speech state channel: VAD sends true on SpeechStarted, false on SpeechEnded.
        // Using mpsc (not watch) so events are queued and never missed,
        // even if the fanout is busy sending audio frames.
        let (speech_state_tx, mut speech_state_rx) = tokio::sync::mpsc::channel::<bool>(32);
        let (asr_done_tx, mut asr_done_rx) = tokio::sync::mpsc::channel::<u64>(8);

        // Fanout + per-utterance ASR spawner.
        // Keeps a small pre-buffer so frames before the watch update are included in the utterance.
        const PRE_BUFFER_FRAMES: usize = 15; // ~300ms to cover min_speech_ms latency
        let fanout_token = cancel_token.child_token();
        let asr_event_tx = event_tx.clone();
        let asr_failure_handler = Arc::clone(&failure_handler);
        let asr_for_fanout = Arc::clone(&asr);
        handles.push(tokio::spawn(async move {
            let mut rx = audio_rx;
            let mut pre_buffer: VecDeque<AudioFrame> = VecDeque::with_capacity(PRE_BUFFER_FRAMES + 1);
            let mut current_asr: Option<ActiveAsrTurn> = None;
            let mut next_turn_id: u64 = 0;

            loop {
                tokio::select! {
                    biased;

                    _ = fanout_token.cancelled() => break,

                    done_turn_id = asr_done_rx.recv() => {
                        match done_turn_id {
                            Some(turn_id) if current_asr.as_ref().map(|turn| turn.turn_id) == Some(turn_id) => {
                                current_asr = None;
                            }
                            Some(_) => {}
                            None => break,
                        }
                    }

                    // Process speech state transitions immediately as they arrive.
                    // Using a separate select arm means we never miss an event
                    // regardless of audio frame timing.
                    speaking = speech_state_rx.recv() => {
                        match speaking {
                            Some(true) => {
                                if let Some(turn) = current_asr.as_mut() {
                                    if turn.capture_open {
                                        continue;
                                    }
                                    turn.cancel();
                                }

                                // Speech started: open utterance channel, pre-fill, spawn ASR.
                                next_turn_id += 1;
                                let turn_id = next_turn_id;
                                let (asr_tx, asr_rx) = tokio::sync::mpsc::channel::<AudioFrame>(200);
                                let asr_clone = Arc::clone(&asr_for_fanout);
                                let ev_tx = asr_event_tx.clone();
                                let handler = Arc::clone(&asr_failure_handler);
                                let asr_cancel = fanout_token.child_token();
                                let asr_done = asr_done_tx.clone();
                                for buffered in pre_buffer.drain(..) {
                                    let _ = asr_tx.try_send(buffered);
                                }
                                current_asr = Some(ActiveAsrTurn {
                                    turn_id,
                                    audio_tx: Some(asr_tx),
                                    cancel_token: asr_cancel.clone(),
                                    capture_open: true,
                                });
                                tokio::spawn(async move {
                                    tracing::info!("ASR provider call started");
                                    let audio_stream = ReceiverAudioStream::new(asr_rx);
                                    let result = tokio::select! {
                                        r = asr_clone.stream(Box::new(audio_stream), ev_tx) => r,
                                        _ = asr_cancel.cancelled() => Ok(()),
                                    };
                                    match &result {
                                        Ok(()) => tracing::info!("ASR provider call completed"),
                                        Err(e) => tracing::warn!(error = %e, "ASR provider call failed"),
                                    }
                                    if let Err(e) = result {
                                        if !matches!(e, common::error::AsrError::ChannelClosed | common::error::AsrError::Cancelled) {
                                            handler.on_provider_failure(Component::Asr, &e);
                                        }
                                    }
                                    let _ = asr_done.send(turn_id).await;
                                });
                            }
                            Some(false) => {
                                // Speech ended: drop sender — ASR drains remaining frames then processes.
                                if let Some(turn) = current_asr.as_mut() {
                                    turn.capture_open = false;
                                    turn.audio_tx = None;
                                }
                            }
                            None => break, // VAD task exited
                        }
                    }

                    // Route audio frames to pre-buffer, active ASR utterance, and VAD.
                    frame = rx.recv() => {
                        let f = match frame {
                            Some(f) => f,
                            None => break,
                        };

                        // Rolling pre-buffer for speech onset recovery.
                        pre_buffer.push_back(f.clone());
                        if pre_buffer.len() > PRE_BUFFER_FRAMES {
                            pre_buffer.pop_front();
                        }

                        // Forward to current ASR utterance if speech is active.
                        if let Some(turn) = current_asr.as_mut() {
                            if turn.capture_open {
                                if let Some(tx) = turn.audio_tx.as_ref() {
                                    let _ = tx.try_send(f.clone());
                                }
                            }
                        }

                        // Forward to VAD. Using send().await provides backpressure
                        // that yields the scheduler, giving VAD task time to run and
                        // fire speech events into speech_state_rx.
                        let _ = vad_audio_tx.send(f).await;
                    }
                }
            }
        }));

        // VAD component: reads audio, emits SpeechStarted/SpeechEnded to event_tx,
        // and updates speech_state_tx for the fanout's ASR gating.
        let vad_token = cancel_token.child_token();
        let vad_event_tx = event_tx.clone();
        let mut vad = VadComponent::new(config.vad_config.clone(), vad_event_tx, vad_token)
            .with_speech_state(speech_state_tx);
        handles.push(tokio::spawn(async move {
            let audio_stream = ReceiverAudioStream::new(vad_audio_rx);
            vad.run(Box::new(audio_stream)).await;
        }));

        // Orchestrator: consumes events, triggers agent/TTS, forwards to egress
        let orch_token = cancel_token.child_token();
        let agent = Arc::new(Mutex::new(AgentCore::new(
            llm.clone(),
            vec![],
            config.system_prompt.clone(),
            CancellationToken::new(),
        )));
        let mut orchestrator = Orchestrator::with_providers_and_agent(
            session_id,
            event_rx,
            event_tx.clone(),
            egress_tx,
            orch_token,
            llm,
            tts,
            std::sync::Arc::clone(&failure_handler),
            Arc::clone(&agent),
        );
        let (turn_count, interrupt_count) = orchestrator.counter_handles();
        handles.push(tokio::spawn(async move {
            orchestrator.run().await;
        }));

        crate::observability::session_started();
        info!(session_id = %session_id, "pipeline session started");

        Ok(Self {
            id: session_id,
            state: SessionState::Active,
            tenant_id: config.tenant_id,
            campaign_id: config.campaign_id,
            started_at: Instant::now(),
            cancel_token,
            task_handles: handles,
            turn_count,
            interrupt_count,
            agent_controller: Some(SessionAgentController { agent }),
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

    /// Start a session using providers derived from AppConfig, with custom agent tools.
    /// Tools are generated from the campaign's `custom_metrics` config (C5).
    pub async fn start_with_config_and_tools(
        app_config: &AppConfig,
        session_config: SessionConfig,
        audio_rx: Receiver<AudioFrame>,
        egress_tx: Sender<PipelineEvent>,
        tools: Vec<Box<dyn agent::tool::Tool>>,
    ) -> Result<Self, SessionError> {
        Self::start_with_config_and_tools_and_memory(
            app_config,
            session_config,
            audio_rx,
            egress_tx,
            tools,
            None,
        )
        .await
    }

    pub async fn start_with_config_and_memory(
        app_config: &AppConfig,
        session_config: SessionConfig,
        audio_rx: Receiver<AudioFrame>,
        egress_tx: Sender<PipelineEvent>,
        memory_backend: Option<std::sync::Arc<dyn agent::memory::ConversationMemoryBackend>>,
    ) -> Result<Self, SessionError> {
        Self::start_with_config_and_tools_and_memory(
            app_config,
            session_config,
            audio_rx,
            egress_tx,
            vec![],
            memory_backend,
        )
        .await
    }

    pub async fn start_with_config_and_tools_and_memory(
        app_config: &AppConfig,
        session_config: SessionConfig,
        audio_rx: Receiver<AudioFrame>,
        egress_tx: Sender<PipelineEvent>,
        tools: Vec<Box<dyn agent::tool::Tool>>,
        memory_backend: Option<std::sync::Arc<dyn agent::memory::ConversationMemoryBackend>>,
    ) -> Result<Self, SessionError> {
        let (asr, llm, tts) = build_providers(app_config, &session_config)?;
        Self::start_with_handler_and_tools_and_memory(
            session_config,
            audio_rx,
            egress_tx,
            asr,
            llm,
            tts,
            Arc::new(LogOnProviderError),
            tools,
            memory_backend,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn start_with_handler_and_tools_and_memory(
        config: SessionConfig,
        audio_rx: Receiver<AudioFrame>,
        egress_tx: Sender<PipelineEvent>,
        asr: Arc<dyn AsrProvider>,
        llm: Arc<dyn LlmProvider>,
        tts: Arc<dyn TtsProvider>,
        failure_handler: Arc<dyn ProviderFailureHandler>,
        tools: Vec<Box<dyn agent::tool::Tool>>,
        memory_backend: Option<std::sync::Arc<dyn agent::memory::ConversationMemoryBackend>>,
    ) -> Result<Self, SessionError> {
        let cancel_token = CancellationToken::new();
        let session_id = config.session_id;

        let (event_tx, event_rx) = tokio::sync::mpsc::channel::<PipelineEvent>(200);
        let mut handles = Vec::new();

        let (vad_audio_tx, vad_audio_rx) = tokio::sync::mpsc::channel::<AudioFrame>(16);
        let (speech_state_tx, mut speech_state_rx) = tokio::sync::mpsc::channel::<bool>(32);
        let (asr_done_tx, mut asr_done_rx) = tokio::sync::mpsc::channel::<u64>(8);

        const PRE_BUFFER_FRAMES: usize = 15;
        let fanout_token = cancel_token.child_token();
        let asr_event_tx = event_tx.clone();
        let asr_failure_handler = Arc::clone(&failure_handler);
        let asr_for_fanout = Arc::clone(&asr);
        handles.push(tokio::spawn(async move {
            let mut rx = audio_rx;
            let mut pre_buffer: VecDeque<AudioFrame> = VecDeque::with_capacity(PRE_BUFFER_FRAMES + 1);
            let mut current_asr: Option<ActiveAsrTurn> = None;
            let mut next_turn_id: u64 = 0;

            loop {
                tokio::select! {
                    biased;

                    _ = fanout_token.cancelled() => break,

                    done_turn_id = asr_done_rx.recv() => {
                        match done_turn_id {
                            Some(turn_id) if current_asr.as_ref().map(|t| t.turn_id) == Some(turn_id) => {
                                current_asr = None;
                            }
                            Some(_) => {}
                            None => break,
                        }
                    }

                    speaking = speech_state_rx.recv() => {
                        match speaking {
                            Some(true) => {
                                if let Some(turn) = current_asr.as_mut() {
                                    if turn.capture_open { continue; }
                                    turn.cancel();
                                }
                                next_turn_id += 1;
                                let turn_id = next_turn_id;
                                let (asr_tx, asr_rx) = tokio::sync::mpsc::channel::<AudioFrame>(200);
                                let asr_clone = Arc::clone(&asr_for_fanout);
                                let ev_tx = asr_event_tx.clone();
                                let handler = Arc::clone(&asr_failure_handler);
                                let asr_cancel = fanout_token.child_token();
                                let asr_done = asr_done_tx.clone();
                                for buffered in pre_buffer.drain(..) {
                                    let _ = asr_tx.try_send(buffered);
                                }
                                current_asr = Some(ActiveAsrTurn {
                                    turn_id,
                                    audio_tx: Some(asr_tx),
                                    cancel_token: asr_cancel.clone(),
                                    capture_open: true,
                                });
                                tokio::spawn(async move {
                                    let audio_stream = ReceiverAudioStream::new(asr_rx);
                                    let result = tokio::select! {
                                        r = asr_clone.stream(Box::new(audio_stream), ev_tx) => r,
                                        _ = asr_cancel.cancelled() => Ok(()),
                                    };
                                    if let Err(e) = result {
                                        if !matches!(e, common::error::AsrError::ChannelClosed | common::error::AsrError::Cancelled) {
                                            handler.on_provider_failure(Component::Asr, &e);
                                        }
                                    }
                                    let _ = asr_done.send(turn_id).await;
                                });
                            }
                            Some(false) => {
                                if let Some(turn) = current_asr.as_mut() {
                                    turn.capture_open = false;
                                    turn.audio_tx = None;
                                }
                            }
                            None => break,
                        }
                    }

                    frame = rx.recv() => {
                        let f = match frame { Some(f) => f, None => break };
                        pre_buffer.push_back(f.clone());
                        if pre_buffer.len() > PRE_BUFFER_FRAMES { pre_buffer.pop_front(); }
                        if let Some(turn) = current_asr.as_mut() {
                            if turn.capture_open {
                                if let Some(tx) = turn.audio_tx.as_ref() {
                                    let _ = tx.try_send(f.clone());
                                }
                            }
                        }
                        let _ = vad_audio_tx.send(f).await;
                    }
                }
            }
        }));

        let vad_token = cancel_token.child_token();
        let vad_event_tx = event_tx.clone();
        let mut vad = VadComponent::new(config.vad_config.clone(), vad_event_tx, vad_token)
            .with_speech_state(speech_state_tx);
        handles.push(tokio::spawn(async move {
            let audio_stream = ReceiverAudioStream::new(vad_audio_rx);
            vad.run(Box::new(audio_stream)).await;
        }));

        let orch_token = cancel_token.child_token();
        let agent_core = match memory_backend {
            Some(memory_backend) => AgentCore::new_with_memory_backend(
                llm.clone(),
                tools,
                config.system_prompt.clone(),
                CancellationToken::new(),
                session_id,
                memory_backend,
            ),
            None => AgentCore::new(
                llm.clone(),
                tools,
                config.system_prompt.clone(),
                CancellationToken::new(),
            ),
        };
        let agent = Arc::new(Mutex::new(agent_core));
        let mut orchestrator = Orchestrator::with_providers_and_agent(
            session_id,
            event_rx,
            event_tx.clone(),
            egress_tx,
            orch_token,
            llm,
            tts,
            Arc::clone(&failure_handler),
            Arc::clone(&agent),
        );
        let (turn_count, interrupt_count) = orchestrator.counter_handles();
        handles.push(tokio::spawn(async move { orchestrator.run().await }));

        crate::observability::session_started();
        info!(session_id = %session_id, "pipeline session started (with tools)");

        Ok(Self {
            id: session_id,
            state: SessionState::Active,
            tenant_id: config.tenant_id,
            campaign_id: config.campaign_id,
            started_at: Instant::now(),
            cancel_token,
            task_handles: handles,
            turn_count,
            interrupt_count,
            agent_controller: Some(SessionAgentController { agent }),
        })
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

    pub async fn terminate(&mut self) -> SessionStats {
        let ended_at = Instant::now();
        if self.state == SessionState::Terminated {
            return SessionStats {
                session_id: self.id,
                tenant_id: self.tenant_id,
                campaign_id: self.campaign_id,
                started_at: self.started_at,
                ended_at,
                turn_count: self.turn_count.load(Ordering::Relaxed),
                interrupt_count: self.interrupt_count.load(Ordering::Relaxed),
            };
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
        let elapsed_ms = self.started_at.elapsed().as_secs_f64() * 1000.0;
        crate::observability::session_ended(elapsed_ms);
        let ended_at = Instant::now();
        let stats = SessionStats {
            session_id: self.id,
            tenant_id: self.tenant_id,
            campaign_id: self.campaign_id,
            started_at: self.started_at,
            ended_at,
            turn_count: self.turn_count.load(Ordering::Relaxed),
            interrupt_count: self.interrupt_count.load(Ordering::Relaxed),
        };
        info!(session_id = %self.id, turn_count = stats.turn_count, interrupt_count = stats.interrupt_count, "pipeline session terminated");
        stats
    }

    pub fn agent_controller(&self) -> Option<SessionAgentController> {
        self.agent_controller.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};

    use common::testing::TestAudioStream;
    use common::types::{Message, ToolDefinition};
    use tokio::sync::mpsc;

    struct FailingAsrProvider {
        attempts: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl AsrProvider for FailingAsrProvider {
        async fn stream(
            &self,
            mut audio: Box<dyn AudioInputStream>,
            _tx: Sender<PipelineEvent>,
        ) -> Result<(), AsrError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            while audio.recv().await.is_some() {}
            Err(AsrError::ConnectionFailed)
        }

        async fn cancel(&self) {}
    }

    struct SuccessfulAsrProvider {
        attempts: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl AsrProvider for SuccessfulAsrProvider {
        async fn stream(
            &self,
            mut audio: Box<dyn AudioInputStream>,
            tx: Sender<PipelineEvent>,
        ) -> Result<(), AsrError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            while audio.recv().await.is_some() {}
            tx.send(PipelineEvent::FinalTranscript {
                text: "fallback transcript".into(),
                language: "en".into(),
            })
            .await
            .map_err(|_| AsrError::ChannelClosed)?;
            Ok(())
        }

        async fn cancel(&self) {}
    }

    struct RecoveringAsrProvider {
        attempts: Arc<AtomicUsize>,
        succeed_on_attempt: usize,
    }

    #[async_trait]
    impl AsrProvider for RecoveringAsrProvider {
        async fn stream(
            &self,
            mut audio: Box<dyn AudioInputStream>,
            tx: Sender<PipelineEvent>,
        ) -> Result<(), AsrError> {
            let attempt = self.attempts.fetch_add(1, Ordering::SeqCst) + 1;
            while audio.recv().await.is_some() {}

            if attempt < self.succeed_on_attempt {
                return Err(AsrError::ConnectionFailed);
            }

            tx.send(PipelineEvent::FinalTranscript {
                text: "recovered transcript".into(),
                language: "en".into(),
            })
            .await
            .map_err(|_| AsrError::ChannelClosed)?;
            Ok(())
        }

        async fn cancel(&self) {}
    }

    struct FailingLlmProvider {
        attempts: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl LlmProvider for FailingLlmProvider {
        async fn stream_completion(
            &self,
            _messages: &[common::types::Message],
            _tools: &[common::types::ToolDefinition],
            _tx: Sender<PipelineEvent>,
        ) -> Result<(), LlmError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            Err(LlmError::ConnectionFailed)
        }

        async fn cancel(&self) {}
    }

    struct SuccessfulLlmProvider {
        attempts: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl LlmProvider for SuccessfulLlmProvider {
        async fn stream_completion(
            &self,
            _messages: &[common::types::Message],
            _tools: &[common::types::ToolDefinition],
            tx: Sender<PipelineEvent>,
        ) -> Result<(), LlmError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            tx.send(PipelineEvent::AgentFinalResponse {
                text: "fallback response".into(),
                tool_calls: vec![],
            })
            .await
            .map_err(|_| LlmError::ConnectionFailed)?;
            Ok(())
        }

        async fn cancel(&self) {}
    }

    #[tokio::test]
    async fn test_asr_fallback_switches_after_primary_retries() {
        let primary_attempts = Arc::new(AtomicUsize::new(0));
        let fallback_attempts = Arc::new(AtomicUsize::new(0));
        let provider = FallbackAsrProvider::new(
            Arc::new(FailingAsrProvider {
                attempts: Arc::clone(&primary_attempts),
            }),
            Some(Arc::new(SuccessfulAsrProvider {
                attempts: Arc::clone(&fallback_attempts),
            })),
        );
        let (tx, mut rx) = mpsc::channel(8);

        provider
            .stream(Box::new(TestAudioStream::silence(40)), tx)
            .await
            .expect("ASR fallback should succeed");

        assert_eq!(primary_attempts.load(Ordering::SeqCst), 3);
        assert_eq!(fallback_attempts.load(Ordering::SeqCst), 1);
        assert!(matches!(
            rx.recv().await,
            Some(PipelineEvent::FinalTranscript { text, .. }) if text == "fallback transcript"
        ));
    }

    #[tokio::test]
    async fn test_asr_primary_retries_even_without_fallback() {
        let primary_attempts = Arc::new(AtomicUsize::new(0));
        let provider = FallbackAsrProvider::new(
            Arc::new(RecoveringAsrProvider {
                attempts: Arc::clone(&primary_attempts),
                succeed_on_attempt: 3,
            }),
            None,
        );
        let (tx, mut rx) = mpsc::channel(8);

        provider
            .stream(Box::new(TestAudioStream::silence(40)), tx)
            .await
            .expect("ASR primary should succeed after retries");

        assert_eq!(primary_attempts.load(Ordering::SeqCst), 3);
        assert!(matches!(
            rx.recv().await,
            Some(PipelineEvent::FinalTranscript { text, .. }) if text == "recovered transcript"
        ));
    }

    #[tokio::test]
    async fn test_llm_fallback_switches_after_primary_retries() {
        let primary_attempts = Arc::new(AtomicUsize::new(0));
        let fallback_attempts = Arc::new(AtomicUsize::new(0));
        let provider = FallbackLlmProvider::new(
            Arc::new(FailingLlmProvider {
                attempts: Arc::clone(&primary_attempts),
            }),
            Some(Arc::new(SuccessfulLlmProvider {
                attempts: Arc::clone(&fallback_attempts),
            })),
        );
        let (tx, mut rx) = mpsc::channel(8);
        let messages = vec![Message::user("hello")];
        let tools: Vec<ToolDefinition> = vec![];

        provider
            .stream_completion(&messages, &tools, tx)
            .await
            .expect("LLM fallback should succeed");

        assert_eq!(primary_attempts.load(Ordering::SeqCst), 2);
        assert_eq!(fallback_attempts.load(Ordering::SeqCst), 1);
        assert!(matches!(
            rx.recv().await,
            Some(PipelineEvent::AgentFinalResponse { text, .. }) if text == "fallback response"
        ));
    }
}
