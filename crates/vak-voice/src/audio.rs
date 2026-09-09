use super::VoiceError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioBlob {
    pub mime: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmSpec {
    pub sample_rate_hz: u32,
    pub channels: u16,
}

pub fn i16_to_f32(input: &[i16]) -> Vec<f32> {
    input.iter().map(|v| *v as f32 / 32768.0).collect()
}
pub fn f32_to_i16(input: &[f32]) -> Vec<i16> {
    input
        .iter()
        .map(|v| (v.clamp(-1.0, 1.0) * 32767.0).round() as i16)
        .collect()
}

/// Linear resampling for speech frames. Provider adapters may replace this
/// with a higher quality implementation, but the deterministic primitive is
/// sufficient for transport normalization and offline operation.
pub fn resample_linear(input: &[i16], from_hz: u32, to_hz: u32) -> Result<Vec<i16>, VoiceError> {
    if from_hz == 0 || to_hz == 0 {
        return Err(VoiceError::InvalidRequest(
            "sample rate must be positive".into(),
        ));
    }
    if input.is_empty() || from_hz == to_hz {
        return Ok(input.to_vec());
    }
    let len = ((input.len() as u64 * to_hz as u64) / from_hz as u64).max(1) as usize;
    let scale = from_hz as f64 / to_hz as f64;
    Ok((0..len)
        .map(|i| {
            let pos = i as f64 * scale;
            let left = pos.floor() as usize;
            let right = (left + 1).min(input.len() - 1);
            let frac = pos - left as f64;
            (input[left.min(input.len() - 1)] as f64 * (1.0 - frac) + input[right] as f64 * frac)
                .round() as i16
        })
        .collect())
}

pub fn wrap_wav(pcm: &[u8], spec: PcmSpec) -> Result<Vec<u8>, VoiceError> {
    if spec.channels == 0 || spec.sample_rate_hz == 0 || pcm.len() % 2 != 0 {
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
mod tests {
    use super::*;
    #[test]
    fn wav_header_is_canonical() {
        let wav = wrap_wav(
            &[0, 0, 255, 127],
            PcmSpec {
                sample_rate_hz: 16_000,
                channels: 1,
            },
        )
        .unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[40..44], &[4, 0, 0, 0]);
    }

    #[test]
    fn resampling_preserves_endpoints() {
        let out = resample_linear(&[0, 1000, 0], 16_000, 8_000).unwrap();
        assert_eq!(out.first(), Some(&0));
        assert_eq!(out.last(), Some(&0));
    }
}
