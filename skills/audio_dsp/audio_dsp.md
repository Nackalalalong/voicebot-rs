# Skill: Audio DSP and Format Handling

Use this whenever dealing with audio data, codec conversion, or VAD/ASR input preparation.

## Canonical internal format

All audio inside the core pipeline is:
- Sample rate: 16000 Hz
- Channels: 1 (mono)
- Bit depth: 16-bit signed integer (i16)
- Frame size: 320 samples = 20ms at 16kHz

**Adapters convert to this format before calling `AudioInputStream::recv()`.**
The core pipeline never handles raw RTP, μ-law, A-law, or float samples.

## Format conversion helpers

```rust
// μ-law → i16 (used in Asterisk adapter)
pub fn ulaw_to_pcm(ulaw_byte: u8) -> i16 {
    let ulaw = !ulaw_byte;
    let sign = ulaw & 0x80;
    let exponent = (ulaw >> 4) & 0x07;
    let mantissa = ulaw & 0x0F;
    let mut sample = ((mantissa as i16) << (exponent + 3)) + (132 << exponent);
    if sign != 0 { sample = -sample; }
    sample
}

// f32 → i16 (used when receiving float PCM from some APIs)
pub fn f32_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

// i16 → f32 (used when sending to APIs that expect float)
pub fn i16_to_f32(sample: i16) -> f32 {
    sample as f32 / i16::MAX as f32
}

// Resample from 8kHz to 16kHz (simple 2x upsample for telephony input)
pub fn upsample_8k_to_16k(input: &[i16]) -> Vec<i16> {
    input.iter().flat_map(|&s| [s, s]).collect()
}

// Resample from 48kHz to 16kHz (3x downsample)
pub fn downsample_48k_to_16k(input: &[i16]) -> Vec<i16> {
    input.chunks(3).map(|chunk| chunk[0]).collect()
}
```

## Building an AudioFrame

```rust
use std::sync::Arc;
use common::audio::AudioFrame;

// From raw Vec<i16>
let frame = AudioFrame {
    data: samples.into(),           // Vec<i16> → Arc<[i16]>
    sample_rate: 16000,
    channels: 1,
    timestamp_ms: elapsed.as_millis() as u64,
};

// From a byte buffer (raw PCM bytes, little-endian i16)
pub fn bytes_to_frame(bytes: &[u8], timestamp_ms: u64) -> AudioFrame {
    assert!(bytes.len() % 2 == 0, "PCM bytes must be even length");
    let samples: Vec<i16> = bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    AudioFrame {
        data: samples.into(),
        sample_rate: 16000,
        channels: 1,
        timestamp_ms,
    }
}

// To bytes (for WebSocket / TTS output)
pub fn frame_to_bytes(frame: &AudioFrame) -> Vec<u8> {
    frame.data.iter()
        .flat_map(|s| s.to_le_bytes())
        .collect()
}
```

## VAD frame sizing

WebRTC VAD requires exact frame sizes. Valid options at 16kHz:
- 160 samples = 10ms
- 320 samples = 20ms  ← preferred
- 480 samples = 30ms

If incoming frames are not exactly 320 samples, buffer and chunk:

```rust
pub struct FrameChunker {
    buffer: Vec<i16>,
    chunk_size: usize,  // 320
}

impl FrameChunker {
    pub fn push(&mut self, samples: &[i16]) -> Vec<Vec<i16>> {
        self.buffer.extend_from_slice(samples);
        let mut chunks = Vec::new();
        while self.buffer.len() >= self.chunk_size {
            chunks.push(self.buffer.drain(..self.chunk_size).collect());
        }
        chunks
    }
}
```

## Energy-based VAD (fallback)

```rust
pub fn rms_energy(samples: &[i16]) -> f32 {
    let sum_sq: f64 = samples.iter()
        .map(|&s| (s as f64).powi(2))
        .sum();
    ((sum_sq / samples.len() as f64).sqrt() / i16::MAX as f64) as f32
}

pub fn is_voiced(samples: &[i16], threshold: f32) -> bool {
    rms_energy(samples) > threshold
}
```

## Silence padding

ASR models need a small silence tail after speech ends to flush their state:

```rust
pub fn silence_frame(duration_ms: u32, timestamp_ms: u64) -> AudioFrame {
    let n_samples = (16000 * duration_ms / 1000) as usize;
    AudioFrame {
        data: vec![0i16; n_samples].into(),
        sample_rate: 16000,
        channels: 1,
        timestamp_ms,
    }
}
```

Send 200ms of silence after `SpeechEnded` to flush Whisper/Deepgram buffers.

## Jitter buffer (Asterisk adapter)

```rust
pub struct JitterBuffer {
    frames: VecDeque<AudioFrame>,
    target_delay_ms: u64,  // 50ms
}

impl JitterBuffer {
    pub fn push(&mut self, frame: AudioFrame) {
        self.frames.push_back(frame);
        while self.frames.len() > (self.target_delay_ms / 20) as usize + 5 {
            self.frames.pop_front();  // drop oldest if overflowing
        }
    }

    pub fn pop(&mut self) -> Option<AudioFrame> {
        if self.frames.len() >= (self.target_delay_ms / 20) as usize {
            self.frames.pop_front()
        } else {
            None  // buffer not full yet, wait
        }
    }
}
```