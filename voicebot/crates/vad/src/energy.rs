/// Compute RMS energy of samples, normalized to 0.0–1.0.
pub fn rms_energy(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples
        .iter()
        .map(|&s| {
            let v = s as f64;
            v * v
        })
        .sum();
    ((sum_sq / samples.len() as f64).sqrt() / i16::MAX as f64) as f32
}

/// Returns true if the RMS energy of samples exceeds the threshold.
pub fn is_voiced(samples: &[i16], threshold: f32) -> bool {
    rms_energy(samples) > threshold
}

/// Buffers incoming samples and chunks them into exact-sized frames.
pub struct FrameChunker {
    buffer: Vec<i16>,
    chunk_size: usize,
}

impl FrameChunker {
    pub fn new(chunk_size: usize) -> Self {
        Self {
            buffer: Vec::new(),
            chunk_size,
        }
    }

    /// Push samples into the buffer, returning any complete chunks.
    pub fn push(&mut self, samples: &[i16]) -> Vec<Vec<i16>> {
        self.buffer.extend_from_slice(samples);
        let mut chunks = Vec::new();
        while self.buffer.len() >= self.chunk_size {
            chunks.push(self.buffer.drain(..self.chunk_size).collect());
        }
        chunks
    }

    /// Push samples and invoke `callback` for each complete chunk without
    /// allocating intermediate `Vec`s.
    pub fn push_with<F: FnMut(&[i16])>(&mut self, samples: &[i16], mut callback: F) {
        self.buffer.extend_from_slice(samples);
        while self.buffer.len() >= self.chunk_size {
            callback(&self.buffer[..self.chunk_size]);
            self.buffer.drain(..self.chunk_size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rms_energy_silence() {
        let samples = vec![0i16; 320];
        assert_eq!(rms_energy(&samples), 0.0);
    }

    #[test]
    fn test_rms_energy_max() {
        let samples = vec![i16::MAX; 320];
        let energy = rms_energy(&samples);
        assert!((energy - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_rms_energy_empty() {
        assert_eq!(rms_energy(&[]), 0.0);
    }

    #[test]
    fn test_is_voiced() {
        let loud: Vec<i16> = (0..320).map(|_| 10000).collect();
        assert!(is_voiced(&loud, 0.02));

        let quiet = vec![0i16; 320];
        assert!(!is_voiced(&quiet, 0.02));
    }

    #[test]
    fn test_frame_chunker_exact() {
        let mut chunker = FrameChunker::new(320);
        let chunks = chunker.push(&vec![1i16; 320]);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 320);
    }

    #[test]
    fn test_frame_chunker_partial() {
        let mut chunker = FrameChunker::new(320);
        let chunks = chunker.push(&vec![1i16; 200]);
        assert_eq!(chunks.len(), 0);

        let chunks = chunker.push(&vec![2i16; 200]);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 320);
    }

    #[test]
    fn test_frame_chunker_multiple() {
        let mut chunker = FrameChunker::new(320);
        let chunks = chunker.push(&vec![1i16; 700]);
        assert_eq!(chunks.len(), 2);
        // 700 - 640 = 60 remaining in buffer
    }
}
