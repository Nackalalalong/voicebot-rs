use common::audio::AudioFrame;
use common::config::{
    AppConfig, AsrConfigGroup, ChannelConfig, LlmConfigGroup, OpenAiConfig, ServerConfig,
    SessionDefaultsConfig, SpeachesAsrConfig, SpeachesTtsConfig, TtsConfigGroup,
};
use common::events::{PipelineEvent, SessionConfig, VadConfig};
use common::types::{AsrProviderType, Language, LlmProviderType, TtsProviderType};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

fn speaches_base_url() -> String {
    std::env::var("SPEACHES_BASE_URL").unwrap_or_else(|_| "http://localhost:8000".into())
}

/// Build an AppConfig that routes ASR and TTS through Speaches,
/// with a stub LLM (OpenAI config with a dummy key).
fn test_app_config() -> AppConfig {
    let base_url = speaches_base_url();
    AppConfig {
        server: ServerConfig {
            host: "127.0.0.1".into(),
            port: 9999,
        },
        session_defaults: SessionDefaultsConfig {
            language: "en".into(),
            asr_provider: "speaches".into(),
            tts_provider: "speaches".into(),
            llm_provider: "openai".into(),
        },
        vad: VadConfig::default(),
        asr: AsrConfigGroup {
            primary: "speaches".into(),
            fallback: None,
            whisper: None,
            speaches: Some(SpeachesAsrConfig {
                base_url: base_url.clone(),
                api_key: None,
                model: "Systran/faster-whisper-small".into(),
                language: Some("en".into()),
            }),
        },
        llm: LlmConfigGroup {
            primary: "openai".into(),
            fallback: None,
            openai: Some(OpenAiConfig {
                base_url: "https://api.openai.com".into(),
                api_key: "stub-key".into(),
                model: "gpt-4".into(),
                max_tokens: 200,
                temperature: 0.7,
            }),
            anthropic: None,
        },
        tts: TtsConfigGroup {
            coqui: None,
            speaches: Some(SpeachesTtsConfig {
                base_url,
                api_key: None,
                model: "kokoro".into(),
                voice: "af_heart".into(),
            }),
        },
        channels: ChannelConfig::default(),
    }
}

/// Helper to create voiced audio frames (sine wave at 440Hz)
fn make_speech_frames(count: usize) -> Vec<AudioFrame> {
    (0..count)
        .map(|i| {
            let samples: Vec<i16> = (0..320)
                .map(|s| {
                    let t = (i * 320 + s) as f32 / 16000.0;
                    (0.5 * (2.0 * std::f32::consts::PI * 440.0 * t).sin() * i16::MAX as f32) as i16
                })
                .collect();
            AudioFrame::new(samples, i as u64 * 20)
        })
        .collect()
}

/// Helper to create silence frames
fn make_silence_frames(count: usize, start_idx: usize) -> Vec<AudioFrame> {
    (0..count)
        .map(|i| AudioFrame::silence(20, (start_idx + i) as u64 * 20))
        .collect()
}

#[tokio::test]
#[ignore = "requires running Speaches server"]
async fn test_pipeline_with_speaches_providers() {
    let app_config = Arc::new(test_app_config());
    let session_id = Uuid::new_v4();

    let session_config = SessionConfig {
        session_id,
        language: Language::English,
        asr_provider: AsrProviderType::Speaches,
        tts_provider: TtsProviderType::Speaches,
        llm_provider: LlmProviderType::OpenAi,
        vad_config: VadConfig::default(),
    };

    let (audio_tx, audio_rx) = mpsc::channel::<AudioFrame>(50);
    let (egress_tx, mut egress_rx) = mpsc::channel::<PipelineEvent>(200);

    let mut session = voicebot_core::session::PipelineSession::start_with_config(
        &app_config,
        session_config,
        audio_rx,
        egress_tx,
    )
    .await
    .expect("session with Speaches providers should start");

    // Send 25 voiced frames (500ms) — triggers VAD SpeechStarted
    for frame in make_speech_frames(25) {
        audio_tx.send(frame).await.unwrap();
    }

    // Send 50 silence frames (1000ms) — triggers VAD SpeechEnded → ASR → Agent
    for frame in make_silence_frames(50, 25) {
        audio_tx.send(frame).await.unwrap();
    }

    // Give the pipeline time to process through VAD → ASR → Agent → TTS
    // Speaches ASR and TTS are slow on CPU, so allow generous timeout
    let mut events = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, egress_rx.recv()).await {
            Ok(Some(event)) => {
                println!("Pipeline event: {event:?}");
                events.push(event);
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    println!("Collected {} pipeline events", events.len());

    session.terminate().await;
}

#[tokio::test]
#[ignore = "requires running Speaches server"]
async fn test_build_providers_speaches() {
    let app_config = test_app_config();
    let session_config = SessionConfig {
        session_id: Uuid::new_v4(),
        language: Language::English,
        asr_provider: AsrProviderType::Speaches,
        tts_provider: TtsProviderType::Speaches,
        llm_provider: LlmProviderType::OpenAi,
        vad_config: VadConfig::default(),
    };

    let (asr, _llm, _tts) = voicebot_core::session::build_providers(&app_config, &session_config)
        .expect("build_providers should succeed");

    // Verify providers were created (basic type check via trait objects)
    // We can't downcast easily, but if we get here without error, it worked
    println!("ASR provider: created");
    println!("LLM provider: created");
    println!("TTS provider: created");

    // Now test ASR with the provider directly
    let (tx, mut rx) = mpsc::channel::<PipelineEvent>(10);
    let audio_stream = common::testing::TestAudioStream::sine(440.0, 1000, 0.5);
    asr.stream(Box::new(audio_stream), tx)
        .await
        .expect("ASR stream should succeed");

    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
        if let PipelineEvent::FinalTranscript { text, .. } = event {
            println!("ASR result: {text:?}");
        }
    }
}
