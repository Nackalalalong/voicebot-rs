use common::events::{PipelineEvent, VadConfig};
use common::traits::AudioInputStream;
use tokio::sync::mpsc::Sender;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::energy::is_voiced;

const FRAME_DURATION_MS: u32 = 20;

pub struct VadComponent {
    config: VadConfig,
    event_tx: Sender<PipelineEvent>,
    cancel_token: CancellationToken,
    /// Optional side-channel for notifying a consumer when speech starts/ends.
    speech_state_tx: Option<Sender<bool>>,
}

impl VadComponent {
    pub fn new(
        config: VadConfig,
        event_tx: Sender<PipelineEvent>,
        cancel_token: CancellationToken,
    ) -> Self {
        Self {
            config,
            event_tx,
            cancel_token,
            speech_state_tx: None,
        }
    }

    /// Attach an mpsc sender that will receive `true` on SpeechStarted
    /// and `false` on SpeechEnded. Use this to gate audio routing.
    pub fn with_speech_state(mut self, tx: Sender<bool>) -> Self {
        self.speech_state_tx = Some(tx);
        self
    }

    pub async fn run(&mut self, mut audio: Box<dyn AudioInputStream>) {
        let mut is_speaking = false;
        let mut voiced_ms: u32 = 0;
        let mut silence_ms: u32 = 0;
        let mut last_timestamp_ms: u64 = 0;

        loop {
            let frame = tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    debug!("VAD cancelled");
                    break;
                }
                frame = audio.recv() => frame,
            };

            let frame = match frame {
                Some(f) => f,
                None => {
                    debug!("Audio stream ended");
                    // If we were speaking, emit SpeechEnded
                    if is_speaking {
                        if let Err(e) = self
                            .event_tx
                            .send(PipelineEvent::SpeechEnded {
                                timestamp_ms: last_timestamp_ms,
                            })
                            .await
                        {
                            warn!("Failed to send SpeechEnded: {}", e);
                        }
                    }
                    break;
                }
            };

            last_timestamp_ms = frame.timestamp_ms;
            let voiced = is_voiced(&frame.data, self.config.energy_threshold);

            if voiced {
                voiced_ms += FRAME_DURATION_MS;
                silence_ms = 0;

                if !is_speaking && voiced_ms >= self.config.min_speech_ms {
                    is_speaking = true;
                    debug!(timestamp_ms = frame.timestamp_ms, "SpeechStarted");
                    if let Some(ref tx) = self.speech_state_tx {
                        if let Err(error) = tx.send(true).await {
                            warn!(%error, "Failed to send speech state start");
                        }
                    }
                    if let Err(e) = self
                        .event_tx
                        .send(PipelineEvent::SpeechStarted {
                            timestamp_ms: frame.timestamp_ms,
                        })
                        .await
                    {
                        warn!("Failed to send SpeechStarted: {}", e);
                        break;
                    }
                }
            } else {
                silence_ms += FRAME_DURATION_MS;

                if is_speaking && silence_ms >= self.config.silence_ms {
                    is_speaking = false;
                    voiced_ms = 0;
                    debug!(timestamp_ms = frame.timestamp_ms, "SpeechEnded");
                    if let Some(ref tx) = self.speech_state_tx {
                        if let Err(error) = tx.send(false).await {
                            warn!(%error, "Failed to send speech state end");
                        }
                    }
                    if let Err(e) = self
                        .event_tx
                        .send(PipelineEvent::SpeechEnded {
                            timestamp_ms: frame.timestamp_ms,
                        })
                        .await
                    {
                        warn!("Failed to send SpeechEnded: {}", e);
                        break;
                    }
                }

                // Reset voiced counter on silence (non-consecutive voiced)
                if !is_speaking {
                    voiced_ms = 0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::testing::TestAudioStream;
    use std::time::Duration;
    use tokio::time::timeout;

    fn default_config() -> VadConfig {
        VadConfig::default()
    }

    #[tokio::test]
    async fn test_vad_emits_speech_started_on_voiced_audio() {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(10);
        let cancel_token = CancellationToken::new();
        let audio = TestAudioStream::sine(440.0, 500, 0.5);

        let mut vad = VadComponent::new(default_config(), event_tx, cancel_token);
        tokio::spawn(async move {
            vad.run(Box::new(audio)).await;
        });

        let event = timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("timeout waiting for SpeechStarted")
            .expect("channel closed");
        assert!(
            matches!(event, PipelineEvent::SpeechStarted { .. }),
            "expected SpeechStarted, got {:?}",
            event
        );
    }

    #[tokio::test]
    async fn test_vad_emits_speech_ended_after_silence() {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(10);
        let cancel_token = CancellationToken::new();
        // 500ms speech + 1000ms silence (silence_ms default is 800)
        let audio = TestAudioStream::speech_then_silence(440.0, 500, 1000, 0.5);

        let mut vad = VadComponent::new(default_config(), event_tx, cancel_token);
        tokio::spawn(async move {
            vad.run(Box::new(audio)).await;
        });

        // First event: SpeechStarted
        let event = timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("timeout waiting for SpeechStarted")
            .expect("channel closed");
        assert!(
            matches!(event, PipelineEvent::SpeechStarted { .. }),
            "expected SpeechStarted, got {:?}",
            event
        );

        // Second event: SpeechEnded
        let event = timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .expect("timeout waiting for SpeechEnded")
            .expect("channel closed");
        assert!(
            matches!(event, PipelineEvent::SpeechEnded { .. }),
            "expected SpeechEnded, got {:?}",
            event
        );
    }

    #[tokio::test]
    async fn test_vad_ignores_short_noise() {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(10);
        let cancel_token = CancellationToken::new();
        // 100ms sine — below min_speech_ms=200
        let audio = TestAudioStream::sine(440.0, 100, 0.5);

        let mut vad = VadComponent::new(default_config(), event_tx, cancel_token);
        tokio::spawn(async move {
            vad.run(Box::new(audio)).await;
        });

        // Should NOT emit SpeechStarted — expect timeout
        let result = timeout(Duration::from_millis(500), event_rx.recv()).await;
        assert!(
            result.is_err() || result.unwrap().is_none(),
            "expected no SpeechStarted for short noise"
        );
    }

    #[tokio::test]
    async fn test_vad_no_speech_ended_without_started() {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(10);
        let cancel_token = CancellationToken::new();
        // Only silence — no speech at all
        let audio = TestAudioStream::silence(1000);

        let mut vad = VadComponent::new(default_config(), event_tx, cancel_token);
        tokio::spawn(async move {
            vad.run(Box::new(audio)).await;
        });

        // Should not emit any events — expect timeout
        let result = timeout(Duration::from_millis(500), event_rx.recv()).await;
        assert!(
            result.is_err() || result.unwrap().is_none(),
            "expected no events for pure silence"
        );
    }

    #[tokio::test]
    async fn test_vad_forwards_speech_state_side_channel() {
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(10);
        let (speech_state_tx, mut speech_state_rx) = tokio::sync::mpsc::channel(10);
        let cancel_token = CancellationToken::new();
        let audio = TestAudioStream::speech_then_silence(440.0, 500, 1000, 0.5);

        let mut vad = VadComponent::new(default_config(), event_tx, cancel_token)
            .with_speech_state(speech_state_tx);
        tokio::spawn(async move {
            vad.run(Box::new(audio)).await;
        });

        let speaking = timeout(Duration::from_secs(2), speech_state_rx.recv())
            .await
            .expect("timeout waiting for speech start")
            .expect("speech state channel closed");
        assert!(speaking, "expected speech start side-channel event");

        let speaking = timeout(Duration::from_secs(2), speech_state_rx.recv())
            .await
            .expect("timeout waiting for speech end")
            .expect("speech state channel closed");
        assert!(!speaking, "expected speech end side-channel event");
    }
}
