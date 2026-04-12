use std::sync::Arc;

use common::audio::AudioFrame;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::AriError;

/// A decoded AudioSocket packet.
pub struct AudioSocketPacket {
    pub kind: u8,
    pub payload: Vec<u8>,
}

/// Read a single AudioSocket framed packet from `reader`.
///
/// Wire format: `[kind: u8][length: u16 BE][payload: bytes]`
pub async fn read_packet<R>(reader: &mut R) -> Result<AudioSocketPacket, AriError>
where
    R: AsyncReadExt + Unpin,
{
    let kind = reader.read_u8().await?;
    let length = reader.read_u16().await?; // big-endian
    let mut payload = vec![0u8; length as usize];
    if length > 0 {
        reader.read_exact(&mut payload).await?;
    }
    Ok(AudioSocketPacket { kind, payload })
}

/// Write an audio packet (kind=0x10) to `writer`.
///
/// `pcm_bytes` must be slin16 PCM (16-bit signed LE, 16kHz, mono).
/// Caller is responsible for splitting into ≤320-byte chunks (10ms each).
pub async fn write_audio_packet<W>(writer: &mut W, pcm_bytes: &[u8]) -> Result<(), AriError>
where
    W: AsyncWriteExt + Unpin,
{
    writer.write_u8(0x10).await?;
    writer.write_u16(pcm_bytes.len() as u16).await?; // big-endian
    writer.write_all(pcm_bytes).await?;
    Ok(())
}

/// Write a hangup packet (kind=0x00) to `writer`.
pub async fn write_hangup_packet<W>(writer: &mut W) -> Result<(), AriError>
where
    W: AsyncWriteExt + Unpin,
{
    writer.write_u8(0x00).await?;
    writer.write_u16(0).await?;
    Ok(())
}

/// Convert raw slin16 PCM bytes into an `AudioFrame`.
///
/// slin16 = 16-bit signed little-endian PCM at 16kHz.
pub fn pcm_bytes_to_frame(bytes: &[u8], timestamp_ms: u64) -> AudioFrame {
    let samples: Vec<i16> = bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    AudioFrame {
        data: Arc::from(samples.as_slice()),
        sample_rate: 16000,
        channels: 1,
        timestamp_ms,
    }
}

/// Convert an `AudioFrame` into raw slin16 PCM bytes.
pub fn frame_to_pcm_bytes(frame: &AudioFrame) -> Vec<u8> {
    frame.data.iter().flat_map(|s| s.to_le_bytes()).collect()
}
