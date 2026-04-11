---
name: Testing Conventions
---

# Skill: Testing Conventions

Use this whenever writing tests for any crate in this project.

## Test structure

```
crates/<name>/
  src/
    lib.rs
    component.rs   ← #[cfg(test)] mod tests { ... } at bottom of each file
  tests/
    integration_test.rs   ← requires real provider credentials or network
```

Unit tests in the same file. Integration tests in `tests/` and gated:

```rust
// Integration tests that need real network — skip in CI without creds
#[tokio::test]
#[ignore = "requires DEEPGRAM_API_KEY"]
async fn test_deepgram_real_audio() { ... }
```

## Required test utilities (implement in `common::testing`)

```rust
// Synthetic audio stream — use in every audio pipeline test
pub struct TestAudioStream {
    frames: VecDeque<AudioFrame>,
    delay_ms: Option<u64>,  // simulate real-time pacing
}

impl TestAudioStream {
    // Stream of silence
    pub fn silence(duration_ms: u32) -> Self {
        let n_frames = duration_ms / 20;
        let frames = (0..n_frames).map(|i| AudioFrame {
            data: vec![0i16; 320].into(),
            sample_rate: 16000,
            channels: 1,
            timestamp_ms: (i * 20) as u64,
        }).collect();
        Self { frames, delay_ms: None }
    }

    // Stream from WAV file (tests/fixtures/audio/*.wav)
    pub fn from_wav(path: &str) -> Self { ... }

    // Sine wave at given frequency (useful for testing VAD thresholds)
    pub fn sine(freq_hz: f32, duration_ms: u32, amplitude: f32) -> Self {
        let n_samples = (16000 * duration_ms / 1000) as usize;
        let samples: Vec<i16> = (0..n_samples)
            .map(|i| {
                let t = i as f32 / 16000.0;
                (amplitude * (2.0 * std::f32::consts::PI * freq_hz * t).sin()
                    * i16::MAX as f32) as i16
            })
            .collect();
        // chunk into 320-sample frames
        let frames = samples.chunks(320).enumerate().map(|(i, chunk)| {
            let mut padded = chunk.to_vec();
            padded.resize(320, 0);
            AudioFrame {
                data: padded.into(),
                sample_rate: 16000,
                channels: 1,
                timestamp_ms: (i * 20) as u64,
            }
        }).collect();
        Self { frames, delay_ms: None }
    }

    // Pace frames at real-time speed (20ms per frame)
    pub fn realtime(mut self) -> Self {
        self.delay_ms = Some(20);
        self
    }
}

impl AudioInputStream for TestAudioStream {
    async fn recv(&mut self) -> Option<AudioFrame> {
        if let Some(delay) = self.delay_ms {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        self.frames.pop_front()
    }
}
```

## VAD tests — must cover

```rust
#[tokio::test]
async fn vad_emits_speech_started_on_voiced_audio() {
    let (event_tx, mut event_rx) = mpsc::channel(10);
    let audio = TestAudioStream::sine(440.0, 500, 0.5);  // 500ms of 440Hz
    let mut vad = VadComponent::new(VadConfig::default(), event_tx);

    tokio::spawn(async move { vad.run(audio).await });

    let event = timeout(Duration::from_secs(2), event_rx.recv()).await
        .expect("timeout").expect("channel closed");
    assert!(matches!(event, PipelineEvent::SpeechStarted { .. }));
}

#[tokio::test]
async fn vad_emits_speech_ended_after_silence() {
    // speech → silence → SpeechEnded
}

#[tokio::test]
async fn vad_ignores_short_noise_bursts() {
    // < min_speech_ms → no SpeechStarted
}
```

## Orchestrator tests — must cover

```rust
#[tokio::test]
async fn orchestrator_transitions_idle_to_listening_on_speech_started() { ... }

#[tokio::test]
async fn orchestrator_cancels_tts_on_interrupt() {
    // Put orchestrator in Speaking state
    // Send Interrupt event
    // Assert: TTS cancel called, state → Idle
}

#[tokio::test]
async fn orchestrator_does_not_drop_final_transcript() {
    // Flood the asr→agent channel
    // Assert: FinalTranscript always delivered
}
```

## Channel backpressure tests

```rust
#[tokio::test]
async fn audio_channel_drops_oldest_on_overflow() {
    let (tx, rx) = mpsc::channel::<AudioFrame>(50);
    // Fill beyond capacity
    for i in 0..60 {
        let frame = make_frame(i as u64);
        match tx.try_send(frame) {
            Ok(_) => {}
            Err(TrySendError::Full(_)) => { /* expected */ }
            Err(e) => panic!("unexpected: {:?}", e),
        }
    }
    // First frame received should have been dropped (oldest)
    // verify by checking timestamps
}
```

## Mock providers

```rust
// Use in orchestrator/core tests to avoid real network calls
pub struct MockAsrProvider {
    pub transcripts: Vec<(String, bool)>,  // (text, is_final)
}

impl AsrProvider for MockAsrProvider {
    async fn stream(
        &self,
        _audio: impl AudioInputStream,
        tx: Sender<PipelineEvent>,
    ) -> Result<(), AsrError> {
        for (text, is_final) in &self.transcripts {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if *is_final {
                tx.send(PipelineEvent::FinalTranscript {
                    text: text.clone(),
                    language: "th".to_string(),
                }).await.ok();
            } else {
                tx.send(PipelineEvent::PartialTranscript {
                    text: text.clone(),
                    confidence: 0.9,
                }).await.ok();
            }
        }
        Ok(())
    }
}

// Similar mocks: MockLlmProvider, MockTtsProvider
```

## Audio fixtures

Place WAV files in `tests/fixtures/audio/`:

- `silence_1s.wav` — 1s of silence
- `speech_thai_hello.wav` — "สวัสดีครับ" in Thai
- `speech_en_hello.wav` — "Hello, how can I help you?" in English
- `speech_interrupted.wav` — speech mid-sentence with abrupt cut

Generate programmatically with `hound` crate if real recordings unavailable.

## Cargo test commands

```bash
# Unit tests only (no network)
cargo test --workspace --exclude '*integration*'

# Specific crate
cargo test -p voicebot-vad

# With real credentials (integration)
DEEPGRAM_API_KEY=sk-... cargo test -p voicebot-asr -- --include-ignored

# With logging
RUST_LOG=debug cargo test -p voicebot-core -- --nocapture
```
