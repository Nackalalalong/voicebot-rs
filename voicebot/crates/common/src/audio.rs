use std::sync::Arc;

/// Canonical audio frame — all audio in the pipeline uses this format.
/// Always 16kHz mono i16.
#[derive(Clone, Debug)]
pub struct AudioFrame {
    /// PCM samples, zero-copy shared ownership
    pub data: Arc<[i16]>,
    /// Always 16000 Hz internally
    pub sample_rate: u32,
    /// Always 1 (mono) internally
    pub channels: u8,
    /// Monotonic ms since session start
    pub timestamp_ms: u64,
}

impl AudioFrame {
    /// Create a new AudioFrame from raw samples.
    pub fn new(data: Vec<i16>, timestamp_ms: u64) -> Self {
        Self {
            data: data.into(),
            sample_rate: 16000,
            channels: 1,
            timestamp_ms,
        }
    }

    /// Create a silence frame of the given duration.
    pub fn silence(duration_ms: u32, timestamp_ms: u64) -> Self {
        let n_samples = (16000 * duration_ms / 1000) as usize;
        Self::new(vec![0i16; n_samples], timestamp_ms)
    }

    /// Number of samples in this frame.
    pub fn num_samples(&self) -> usize {
        self.data.len()
    }

    /// Duration of this frame in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        (self.data.len() as u64 * 1000) / self.sample_rate as u64
    }

    /// Build from raw PCM bytes (little-endian i16).
    pub fn from_pcm_bytes(bytes: &[u8], timestamp_ms: u64) -> Self {
        let samples: Vec<i16> = bytes
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        Self::new(samples, timestamp_ms)
    }

    /// Convert to raw PCM bytes (little-endian i16).
    pub fn to_pcm_bytes(&self) -> Vec<u8> {
        self.data.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    /// Append PCM bytes (little-endian i16) directly into an existing buffer.
    /// Avoids allocating a transient `Vec<u8>` in hot audio paths.
    pub fn append_pcm_bytes_to(&self, buf: &mut Vec<u8>) {
        buf.reserve(self.data.len() * 2);
        for &s in &*self.data {
            buf.extend_from_slice(&s.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_frame_new() {
        let frame = AudioFrame::new(vec![0i16; 320], 0);
        assert_eq!(frame.sample_rate, 16000);
        assert_eq!(frame.channels, 1);
        assert_eq!(frame.num_samples(), 320);
        assert_eq!(frame.duration_ms(), 20);
    }

    #[test]
    fn test_audio_frame_silence() {
        let frame = AudioFrame::silence(20, 100);
        assert_eq!(frame.num_samples(), 320);
        assert_eq!(frame.timestamp_ms, 100);
        assert!(frame.data.iter().all(|&s| s == 0));
    }

    #[test]
    fn test_pcm_bytes_roundtrip() {
        let original = AudioFrame::new(vec![1, -1, 32767, -32768], 0);
        let bytes = original.to_pcm_bytes();
        let restored = AudioFrame::from_pcm_bytes(&bytes, 0);
        assert_eq!(&*original.data, &*restored.data);
    }
}
