use common::audio::AudioFrame;

const RTP_HEADER_LEN: usize = 12;
const RTP_PAYLOAD_TYPE_PCMU: u8 = 0;
const MU_LAW_BIAS: i16 = 0x84;
const MU_LAW_CLIP: i16 = 32_635;
const MU_LAW_SEG_END: [i16; 8] = [
    0x00FF, 0x01FF, 0x03FF, 0x07FF, 0x0FFF, 0x1FFF, 0x3FFF, 0x7FFF,
];

pub fn build_pcmu_packet(samples_16k: &[i16], sequence: u16, timestamp: u32, ssrc: u32) -> Vec<u8> {
    let ulaw_samples = downsample_16k_to_8k(samples_16k);
    let mut packet = Vec::with_capacity(RTP_HEADER_LEN + ulaw_samples.len());
    packet.push(0x80);
    packet.push(RTP_PAYLOAD_TYPE_PCMU);
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.extend_from_slice(&timestamp.to_be_bytes());
    packet.extend_from_slice(&ssrc.to_be_bytes());
    packet.extend(ulaw_samples.into_iter().map(linear_to_mulaw));
    packet
}

pub fn parse_rtp_payload(packet: &[u8]) -> Option<&[u8]> {
    if packet.len() < RTP_HEADER_LEN {
        return None;
    }
    if packet[0] >> 6 != 2 {
        return None;
    }

    let csrc_count = (packet[0] & 0x0F) as usize;
    let has_extension = packet[0] & 0x10 != 0;
    let has_padding = packet[0] & 0x20 != 0;

    let mut offset = RTP_HEADER_LEN + (csrc_count * 4);
    if packet.len() < offset {
        return None;
    }

    if has_extension {
        if packet.len() < offset + 4 {
            return None;
        }
        let extension_length_words =
            u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]) as usize;
        offset += 4 + (extension_length_words * 4);
        if packet.len() < offset {
            return None;
        }
    }

    let payload_end = if has_padding {
        let padding = *packet.last()? as usize;
        packet.len().checked_sub(padding)?
    } else {
        packet.len()
    };

    if payload_end < offset {
        return None;
    }

    Some(&packet[offset..payload_end])
}

pub fn pcmu_payload_to_frame(payload: &[u8], timestamp_ms: u64) -> AudioFrame {
    let decoded_8k: Vec<i16> = payload.iter().copied().map(mulaw_to_linear).collect();
    AudioFrame::new(upsample_8k_to_16k(&decoded_8k), timestamp_ms)
}

fn downsample_16k_to_8k(samples: &[i16]) -> Vec<i16> {
    samples.iter().step_by(2).copied().collect()
}

fn upsample_8k_to_16k(samples: &[i16]) -> Vec<i16> {
    let mut upsampled = Vec::with_capacity(samples.len() * 2);
    for &sample in samples {
        upsampled.push(sample);
        upsampled.push(sample);
    }
    upsampled
}

fn linear_to_mulaw(sample: i16) -> u8 {
    let mut magnitude = sample;
    let sign_mask = if magnitude < 0 {
        magnitude = MU_LAW_BIAS.saturating_sub(magnitude);
        0x7F
    } else {
        magnitude = magnitude.saturating_add(MU_LAW_BIAS);
        0xFF
    };
    magnitude = magnitude.min(MU_LAW_CLIP);

    let segment = MU_LAW_SEG_END
        .iter()
        .position(|&segment_end| magnitude <= segment_end)
        .unwrap_or(MU_LAW_SEG_END.len());
    if segment >= MU_LAW_SEG_END.len() {
        return 0x7F ^ sign_mask;
    }

    let mantissa = ((magnitude >> (segment + 3)) & 0x0F) as u8;
    (((segment as u8) << 4) | mantissa) ^ sign_mask
}

fn mulaw_to_linear(encoded: u8) -> i16 {
    let inverted = !encoded;
    let mut magnitude = (((inverted & 0x0F) as i16) << 3) + MU_LAW_BIAS;
    magnitude <<= ((inverted & 0x70) >> 4) as usize;

    if inverted & 0x80 != 0 {
        MU_LAW_BIAS.saturating_sub(magnitude)
    } else {
        magnitude.saturating_sub(MU_LAW_BIAS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_rtp_payload() {
        let packet = build_pcmu_packet(&[0; 320], 1, 160, 42);
        let payload = parse_rtp_payload(&packet).expect("payload");

        assert_eq!(payload.len(), 160);
    }

    #[test]
    fn converts_pcmu_payload_back_to_frame() {
        let packet = build_pcmu_packet(&[500; 320], 1, 160, 42);
        let payload = parse_rtp_payload(&packet).expect("payload");
        let frame = pcmu_payload_to_frame(payload, 40);

        assert_eq!(frame.timestamp_ms, 40);
        assert_eq!(frame.sample_rate, 16_000);
        assert_eq!(frame.num_samples(), 320);
    }
}
