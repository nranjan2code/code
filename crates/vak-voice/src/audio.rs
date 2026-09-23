use super::VoiceError;

/// The socket's only audio shape: mono 16-bit little-endian PCM at 16 kHz.
pub const SOCKET_SAMPLE_RATE_HZ: u32 = 16_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmSpec {
    pub sample_rate_hz: u32,
    pub channels: u16,
}

impl PcmSpec {
    pub const SOCKET: Self = Self {
        sample_rate_hz: SOCKET_SAMPLE_RATE_HZ,
        channels: 1,
    };
}

pub fn wrap_wav(pcm: &[u8], spec: PcmSpec) -> Result<Vec<u8>, VoiceError> {
    if spec.channels == 0 || spec.sample_rate_hz == 0 || !pcm.len().is_multiple_of(2) {
        return Err(VoiceError::InvalidRequest("invalid PCM stream".into()));
    }
    let data_len = u32::try_from(pcm.len())
        .map_err(|_| VoiceError::InvalidRequest("audio too large".into()))?;
    let byte_rate = spec.sample_rate_hz * u32::from(spec.channels) * 2;
    let block_align = spec.channels * 2;
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&spec.channels.to_le_bytes());
    out.extend_from_slice(&spec.sample_rate_hz.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn wav_header_is_canonical() {
        let wav = wrap_wav(&[0, 0, 255, 127], PcmSpec::SOCKET).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[40..44], &[4, 0, 0, 0]);
    }
}
