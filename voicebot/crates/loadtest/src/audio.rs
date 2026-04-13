use std::path::Path;

use serde::Serialize;

use crate::error::LoadtestError;

pub const CANONICAL_SAMPLE_RATE: u32 = 16_000;
pub const CANONICAL_CHANNELS: u16 = 1;
pub const TEN_MS_SAMPLES: usize = 160;

#[derive(Debug, Clone, Serialize)]
pub struct NormalizedAudio {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

impl NormalizedAudio {
    pub fn duration_ms(&self) -> u64 {
        (self.samples.len() as u64 * 1000) / self.sample_rate as u64
    }
}

pub fn load_and_normalize_wav(path: &Path) -> Result<NormalizedAudio, LoadtestError> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();

    if spec.sample_format != hound::SampleFormat::Int || spec.bits_per_sample != 16 {
        return Err(LoadtestError::UnsupportedWav {
            path: path.to_path_buf(),
            reason: format!(
                "expected 16-bit PCM integer WAV, got {:?} {}-bit",
                spec.sample_format, spec.bits_per_sample
            ),
        });
    }

    if spec.channels != 1 && spec.channels != 2 {
        return Err(LoadtestError::UnsupportedWav {
            path: path.to_path_buf(),
            reason: format!(
                "expected mono or stereo WAV, got {} channels",
                spec.channels
            ),
        });
    }

    let raw_samples: Vec<i16> = reader.samples::<i16>().collect::<Result<Vec<_>, _>>()?;

    let mono_samples = if spec.channels == 1 {
        raw_samples
    } else {
        raw_samples
            .chunks_exact(2)
            .map(|pair| ((pair[0] as i32 + pair[1] as i32) / 2) as i16)
            .collect()
    };

    let normalized_samples = match spec.sample_rate {
        CANONICAL_SAMPLE_RATE => mono_samples,
        8_000 => upsample_8k_to_16k(&mono_samples),
        other => resample_to_16k(&mono_samples, other),
    };

    Ok(NormalizedAudio {
        sample_rate: CANONICAL_SAMPLE_RATE,
        channels: CANONICAL_CHANNELS,
        samples: normalized_samples,
    })
}

pub fn write_wav(path: &Path, samples: &[i16]) -> Result<(), LoadtestError> {
    let spec = hound::WavSpec {
        channels: CANONICAL_CHANNELS,
        sample_rate: CANONICAL_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &sample in samples {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    Ok(())
}

pub fn samples_to_pcm_bytes(samples: &[i16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for &sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

pub fn upsample_8k_to_16k(input: &[i16]) -> Vec<i16> {
    let mut output = Vec::with_capacity(input.len() * 2);
    for &sample in input {
        output.push(sample);
        output.push(sample);
    }
    output
}

pub fn downsample_16k_to_8k(input: &[i16]) -> Vec<i16> {
    input.iter().step_by(2).copied().collect()
}

fn resample_to_16k(input: &[i16], input_rate: u32) -> Vec<i16> {
    if input.is_empty() || input_rate == CANONICAL_SAMPLE_RATE {
        return input.to_vec();
    }

    let output_len =
        ((input.len() as f64 * CANONICAL_SAMPLE_RATE as f64) / input_rate as f64).round() as usize;
    let mut output = Vec::with_capacity(output_len);

    for output_index in 0..output_len {
        let source_position =
            output_index as f64 * input_rate as f64 / CANONICAL_SAMPLE_RATE as f64;
        let left_index = source_position.floor() as usize;
        let right_index = (left_index + 1).min(input.len() - 1);
        let fraction = source_position - left_index as f64;

        let interpolated =
            input[left_index] as f64 * (1.0 - fraction) + input[right_index] as f64 * fraction;
        output.push(interpolated.round() as i16);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn normalizes_stereo_8k_wav_to_16k_mono() {
        let file = NamedTempFile::new().expect("temp file");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(file.path(), spec).expect("create wav");
        for sample in [100i16, 300, -200, 200, 0, 400] {
            writer.write_sample(sample).expect("write sample");
        }
        writer.finalize().expect("finalize wav");

        let normalized = load_and_normalize_wav(file.path()).expect("normalize wav");

        assert_eq!(normalized.sample_rate, 16_000);
        assert_eq!(normalized.channels, 1);
        assert_eq!(normalized.samples, vec![200, 200, 0, 0, 200, 200]);
    }

    #[test]
    fn normalizes_24k_wav_to_16k_mono() {
        let file = NamedTempFile::new().expect("temp file");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 24_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(file.path(), spec).expect("create wav");
        for sample in [0i16, 300, 600] {
            writer.write_sample(sample).expect("write sample");
        }
        writer.finalize().expect("finalize wav");

        let normalized = load_and_normalize_wav(file.path()).expect("normalize wav");

        assert_eq!(normalized.sample_rate, 16_000);
        assert_eq!(normalized.channels, 1);
        assert_eq!(normalized.samples.len(), 2);
    }
}
