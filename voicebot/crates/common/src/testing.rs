use crate::audio::AudioFrame;
use crate::traits::AudioInputStream;
use async_trait::async_trait;
use std::collections::VecDeque;
use std::time::Duration;

/// A synthetic audio stream for testing. Supports silence, sine waves, and WAV loading.
pub struct TestAudioStream {
    frames: VecDeque<AudioFrame>,
    delay_ms: Option<u64>,
}

impl TestAudioStream {
    /// Create a stream of silence for the given duration.
    pub fn silence(duration_ms: u32) -> Self {
        let n_frames = duration_ms / 20;
        let frames = (0..n_frames)
            .map(|i| AudioFrame {
                data: vec![0i16; 320].into(),
                sample_rate: 16000,
                channels: 1,
                timestamp_ms: (i * 20) as u64,
            })
            .collect();
        Self {
            frames,
            delay_ms: None,
        }
    }

    /// Create a sine wave at the given frequency and amplitude.
    pub fn sine(freq_hz: f32, duration_ms: u32, amplitude: f32) -> Self {
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
                    timestamp_ms: (i * 20) as u64,
                }
            })
            .collect();

        Self {
            frames,
            delay_ms: None,
        }
    }

    /// Create a stream that alternates speech (sine) then silence.
    pub fn speech_then_silence(
        freq_hz: f32,
        speech_ms: u32,
        silence_ms: u32,
        amplitude: f32,
    ) -> Self {
        let mut speech = Self::sine(freq_hz, speech_ms, amplitude);
        let silence = Self::silence(silence_ms);

        // Adjust silence timestamps to follow speech
        let speech_end_ts = speech
            .frames
            .back()
            .map(|f| f.timestamp_ms + 20)
            .unwrap_or(0);
        for mut frame in silence.frames {
            frame.timestamp_ms += speech_end_ts;
            speech.frames.push_back(frame);
        }
        speech
    }

    /// Enable real-time pacing (20ms per frame).
    pub fn realtime(mut self) -> Self {
        self.delay_ms = Some(20);
        self
    }

    /// Total number of frames in the stream.
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }
}

#[async_trait]
impl AudioInputStream for TestAudioStream {
    async fn recv(&mut self) -> Option<AudioFrame> {
        if let Some(delay) = self.delay_ms {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        self.frames.pop_front()
    }
}

/// Adapter: wrap a tokio mpsc::Receiver as an AudioInputStream.
pub struct ReceiverAudioStream {
    rx: tokio::sync::mpsc::Receiver<AudioFrame>,
}

impl ReceiverAudioStream {
    pub fn new(rx: tokio::sync::mpsc::Receiver<AudioFrame>) -> Self {
        Self { rx }
    }
}

#[async_trait]
impl AudioInputStream for ReceiverAudioStream {
    async fn recv(&mut self) -> Option<AudioFrame> {
        self.rx.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_silence_stream() {
        let mut stream = TestAudioStream::silence(100); // 5 frames
        let mut count = 0;
        while let Some(frame) = stream.recv().await {
            assert_eq!(frame.num_samples(), 320);
            assert!(frame.data.iter().all(|&s| s == 0));
            count += 1;
        }
        assert_eq!(count, 5);
    }

    #[tokio::test]
    async fn test_sine_stream_has_energy() {
        let mut stream = TestAudioStream::sine(440.0, 100, 0.5);
        let frame = stream.recv().await.unwrap();
        let max_sample = frame.data.iter().map(|s| s.abs()).max().unwrap();
        assert!(max_sample > 1000, "sine wave should have significant energy");
    }

    #[tokio::test]
    async fn test_speech_then_silence() {
        let mut stream = TestAudioStream::speech_then_silence(440.0, 100, 100, 0.5);
        assert_eq!(stream.frame_count(), 10); // 5 speech + 5 silence
        // First frames should have energy
        let first = stream.recv().await.unwrap();
        let max = first.data.iter().map(|s| s.abs()).max().unwrap();
        assert!(max > 0);
    }
}
