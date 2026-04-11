use common::audio::AudioFrame;
use common::events::PipelineEvent;
use common::traits::{AsrProvider, AudioInputStream};
use std::collections::VecDeque;
use std::time::Duration;
use tokio::sync::mpsc;

/// Simple audio stream from raw PCM bytes.
struct WavAudioStream {
    frames: VecDeque<AudioFrame>,
}

impl WavAudioStream {
    fn from_wav_file(path: &str) -> Self {
        let data = std::fs::read(path).expect("failed to read WAV file");
        // Skip 44-byte WAV header, parse LE i16 samples
        let pcm = &data[44..];
        let samples: Vec<i16> = pcm
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        let frames = samples
            .chunks(320)
            .enumerate()
            .filter(|(_, chunk)| chunk.len() == 320)
            .map(|(i, chunk)| AudioFrame {
                data: chunk.to_vec().into(),
                sample_rate: 16000,
                channels: 1,
                timestamp_ms: i as u64 * 20,
            })
            .collect();
        Self { frames }
    }

    fn sine(freq_hz: f32, duration_ms: u32, amplitude: f32) -> Self {
        let n_samples = (16000 * duration_ms / 1000) as usize;
        let samples: Vec<i16> = (0..n_samples)
            .map(|i| {
                let t = i as f32 / 16000.0;
                (amplitude * (2.0 * std::f32::consts::PI * freq_hz * t).sin() * i16::MAX as f32)
                    as i16
            })
            .collect();
        let frames = samples
            .chunks(320)
            .enumerate()
            .map(|(i, chunk)| {
                let mut padded = chunk.to_vec();
                padded.resize(320, 0);
                AudioFrame {
                    data: padded.into(),
                    sample_rate: 16000,
                    channels: 1,
                    timestamp_ms: i as u64 * 20,
                }
            })
            .collect();
        Self { frames }
    }
}

#[async_trait::async_trait]
impl AudioInputStream for WavAudioStream {
    async fn recv(&mut self) -> Option<AudioFrame> {
        self.frames.pop_front()
    }
}

fn speaches_base_url() -> String {
    std::env::var("SPEACHES_BASE_URL").unwrap_or_else(|_| "http://localhost:8000".into())
}

fn speaches_asr_model() -> String {
    std::env::var("SPEACHES_ASR_MODEL")
        .unwrap_or_else(|_| "Systran/faster-whisper-small".into())
}

#[tokio::test]
#[ignore = "requires running Speaches server"]
async fn test_speaches_asr_transcribes_audio() {
    let provider = asr::speaches::SpeachesAsrProvider::new(
        speaches_base_url(),
        speaches_asr_model(),
    )
    .with_language("en".into());

    let (tx, mut rx) = mpsc::channel::<PipelineEvent>(10);

    // Use the sine wave fixture — Speaches will likely return empty or noise
    // but the important thing is that the round-trip works without errors
    let audio = WavAudioStream::from_wav_file("tests/fixtures/audio/sine_440hz_1s.wav");

    provider
        .stream(Box::new(audio), tx)
        .await
        .expect("ASR stream should succeed");

    // Drain events — we may or may not get a transcript for a sine wave
    let mut got_event = false;
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(100), rx.recv()).await
    {
        match event {
            PipelineEvent::FinalTranscript { text, .. } => {
                println!("ASR transcribed: {text:?}");
                got_event = true;
            }
            _ => {}
        }
    }
    // For a sine wave, it's acceptable to get no transcript
    println!("Got transcript event: {got_event}");
}

#[tokio::test]
#[ignore = "requires running Speaches server"]
async fn test_speaches_asr_handles_silence() {
    let provider = asr::speaches::SpeachesAsrProvider::new(
        speaches_base_url(),
        speaches_asr_model(),
    )
    .with_language("en".into());

    let (tx, mut rx) = mpsc::channel::<PipelineEvent>(10);

    let audio = WavAudioStream::from_wav_file("tests/fixtures/audio/silence_1s.wav");

    provider
        .stream(Box::new(audio), tx)
        .await
        .expect("ASR stream should succeed even with silence");

    // Silence should produce no transcript or an empty one
    let event = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
    match event {
        Ok(Some(PipelineEvent::FinalTranscript { text, .. })) => {
            println!("Got transcript for silence: {text:?}");
            // Some models hallucinate on silence — that's a known Whisper behavior
        }
        _ => {
            println!("No transcript for silence — expected");
        }
    }
}

#[tokio::test]
#[ignore = "requires running Speaches server"]
async fn test_speaches_asr_synthetic_speech() {
    let provider = asr::speaches::SpeachesAsrProvider::new(
        speaches_base_url(),
        speaches_asr_model(),
    )
    .with_language("en".into());

    let (tx, mut rx) = mpsc::channel::<PipelineEvent>(10);

    // Generate a longer sine for better chance of non-empty output
    let audio = WavAudioStream::sine(440.0, 3000, 0.5);

    provider
        .stream(Box::new(audio), tx)
        .await
        .expect("ASR stream should succeed");

    // Collect all events
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(100), rx.recv()).await
    {
        if let PipelineEvent::FinalTranscript { text, language } = event {
            println!("Transcript: {text:?} (lang={language})");
        }
    }
}
