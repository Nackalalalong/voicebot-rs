---
name: rubato-resample
description: >
  Use this skill whenever the user needs to resample audio in Rust — changing sample rate,
  normalizing mic input to 16 kHz for ASR, handling variable-rate streams, or converting
  between 8/16/44.1/48 kHz. Trigger on: rubato, resample, sample rate, 16kHz, 48kHz,
  SincFixedIn, SincFixedOut, rate conversion, audio format normalization.
---

# rubato for Audio Resampling

## Setup

```toml
[dependencies]
rubato = "0.15"
```

---

## Which resampler to use

| Scenario | Use |
|---|---|
| Known input size, variable output | `SincFixedIn` ← most common for voicebot |
| Known output size, variable input | `SincFixedOut` |
| Realtime, low latency priority | `FastFixedIn` |
| Highest quality offline | `SincFixedIn` with `SincInterpolationParameters` |

---

## Patterns

### Normalize mic input to 16 kHz (standard voicebot pattern)

```rust
use rubato::{SincFixedIn, SincInterpolationParameters, SincInterpolationType, Resampler, WindowFunction};

pub fn build_resampler(from_sr: f64, to_sr: f64) -> SincFixedIn<f32> {
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };

    SincFixedIn::<f32>::new(
        to_sr / from_sr,   // ratio: e.g. 16000.0 / 48000.0
        2.0,               // max relative ratio (for variable-rate)
        params,
        1024,              // chunk size (input frames per call)
        1,                 // channels
    ).expect("failed to build resampler")
}
```

### Process a chunk

rubato expects `Vec<Vec<f32>>` — outer vec is channels, inner is samples.

```rust
pub fn resample_chunk(
    resampler: &mut SincFixedIn<f32>,
    input: &[f32],  // mono PCM from cpal
) -> Vec<f32> {
    // Wrap in channel vec
    let input_chunk = vec![input.to_vec()];

    let output = resampler.process(&input_chunk, None)
        .expect("resample failed");

    // output[0] is the mono channel
    output.into_iter().next().unwrap_or_default()
}
```

### Streaming pipeline (cpal → rubato → processing)

```rust
use crossbeam::channel::{bounded, Receiver, Sender};

// In your cpal callback: push raw 48kHz chunks
let (tx_raw, rx_raw): (Sender<Vec<f32>>, Receiver<Vec<f32>>) = bounded(16);

// In processing thread:
let mut resampler = build_resampler(48_000.0, 16_000.0);
let mut input_buf: Vec<f32> = Vec::new();
let chunk_size = resampler.input_frames_next(); // how many frames rubato wants

for raw in rx_raw {
    input_buf.extend_from_slice(&raw);

    while input_buf.len() >= chunk_size {
        let chunk: Vec<f32> = input_buf.drain(..chunk_size).collect();
        let resampled = resample_chunk(&mut resampler, &chunk);
        // → pass resampled to ndarray framing
    }
}
```

### Multi-channel (stereo → mono then resample)

```rust
// Downmix stereo to mono first
pub fn stereo_to_mono(stereo: &[f32]) -> Vec<f32> {
    stereo.chunks(2).map(|s| (s[0] + s[1]) * 0.5).collect()
}

// Then resample the mono signal
let mono = stereo_to_mono(&stereo_samples);
let resampled = resample_chunk(&mut resampler, &mono);
```

---

## Chunk size maths

rubato processes fixed-size chunks. Always query `input_frames_next()` before each call — it can change slightly after a call (by ±1 sample) due to rounding.

```rust
let needed = resampler.input_frames_next();
// buffer until you have `needed` samples before calling process()
```

---

## Common mistakes

- **Wrong ratio direction**: ratio = `target_sr / source_sr`. For 48k→16k: `16000.0/48000.0 = 0.333`. Swapping gives 3× speed playback.
- **Forgetting channel wrapping**: rubato always takes `Vec<Vec<f32>>`. A plain `Vec<f32>` passed directly will fail — wrap it: `vec![samples]`.
- **Not draining input_buf**: if you pass fewer frames than `input_frames_next()`, rubato returns an error. Always buffer until you have enough.
- **Resampling after framing**: resample raw PCM *before* framing and windowing, not after.