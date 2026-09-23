//! Speech evidence for a closed utterance.
//!
//! The client decides where an utterance starts and stops, but the server
//! decides whether it is worth a paid transcription call and an Agent turn.
//! A fixed loudness threshold cannot make that call: in a noisy room every
//! frame clears it. Evidence is therefore measured against the utterance's
//! own quiet floor, so steady room noise — however loud — carries no speech.
//!
//! `crates/vak-client-ui/src/voice-activity.ts` applies the same constants
//! live for endpointing; `constants_match_the_client_detector` pins them.

/// Analysis frame length.
pub const FRAME_MS: u32 = 20;
/// Normalised RMS below which a frame is silence in any room.
pub const ABSOLUTE_SPEECH_RMS: f32 = 0.018;
/// A voiced frame must stand this far above the quiet floor (about 8 dB).
pub const FLOOR_RATIO: f32 = 2.5;
/// Voiced audio an utterance needs before it may reach a provider.
pub const MIN_VOICED_MS: u32 = 240;
/// Share of frames, from the quietest, that define the floor.
pub const FLOOR_PERCENTILE: f32 = 0.2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeechEvidence {
    pub voiced_ms: u32,
    pub total_ms: u32,
    /// Normalised RMS of the utterance's quiet floor.
    pub floor_rms: f32,
}

impl SpeechEvidence {
    /// Measure mono 16-bit little-endian PCM.
    pub fn measure(pcm: &[u8], sample_rate_hz: u32) -> Self {
        let frame_samples = (sample_rate_hz * FRAME_MS / 1000).max(1) as usize;
        let samples: Vec<f32> = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32768.0)
            .collect();
        let mut frames: Vec<f32> = samples
            .chunks(frame_samples)
            .filter(|frame| frame.len() == frame_samples)
            .map(|frame| (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt())
            .collect();
        let total_ms = frames.len() as u32 * FRAME_MS;
        if frames.is_empty() {
            return Self {
                voiced_ms: 0,
                total_ms,
                floor_rms: 0.0,
            };
        }
        let rms = frames.clone();
        frames.sort_by(f32::total_cmp);
        let floor_index = ((frames.len() - 1) as f32 * FLOOR_PERCENTILE) as usize;
        let floor_rms = frames[floor_index];
        let threshold = ABSOLUTE_SPEECH_RMS.max(floor_rms * FLOOR_RATIO);
        let voiced = rms.iter().filter(|value| **value >= threshold).count() as u32;
        Self {
            voiced_ms: voiced * FRAME_MS,
            total_ms,
            floor_rms,
        }
    }

    pub fn is_speech(&self) -> bool {
        self.voiced_ms >= MIN_VOICED_MS
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;

    /// Deterministic PCM: `segments` of (milliseconds, amplitude, hz).
    fn pcm(segments: &[(u32, f32, f32)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut n = 0u32;
        for &(ms, amplitude, hz) in segments {
            for _ in 0..(RATE * ms / 1000) {
                let t = n as f32 / RATE as f32;
                // A tone plus a fixed pseudo-noise term, so "noise" segments
                // are broadband rather than silent.
                let noise = (((n.wrapping_mul(1_103_515_245).wrapping_add(12_345)) >> 16) & 0x7fff)
                    as f32
                    / 16_384.0
                    - 1.0;
                let value =
                    amplitude * (0.7 * (std::f32::consts::TAU * hz * t).sin() + 0.3 * noise);
                out.extend_from_slice(&((value * 32767.0) as i16).to_le_bytes());
                n += 1;
            }
        }
        out
    }

    #[test]
    fn speech_followed_by_a_quiet_tail_is_speech() {
        let evidence =
            SpeechEvidence::measure(&pcm(&[(900, 0.25, 220.0), (1500, 0.004, 0.0)]), RATE);
        assert!(evidence.is_speech(), "{evidence:?}");
        assert!(evidence.voiced_ms >= 800);
    }

    #[test]
    fn loud_steady_room_noise_is_not_speech() {
        // Far above the absolute threshold, but flat: no frame stands out
        // from the floor, which is what a fan, traffic or a crowd looks like.
        let evidence = SpeechEvidence::measure(&pcm(&[(2500, 0.12, 0.0)]), RATE);
        assert!(!evidence.is_speech(), "{evidence:?}");
    }

    #[test]
    fn a_click_is_not_speech() {
        let evidence =
            SpeechEvidence::measure(&pcm(&[(100, 0.5, 300.0), (1500, 0.003, 0.0)]), RATE);
        assert!(!evidence.is_speech(), "{evidence:?}");
    }

    #[test]
    fn speech_over_room_noise_is_still_speech() {
        let evidence = SpeechEvidence::measure(
            &pcm(&[(400, 0.03, 0.0), (900, 0.3, 180.0), (1500, 0.03, 0.0)]),
            RATE,
        );
        assert!(evidence.is_speech(), "{evidence:?}");
    }

    #[test]
    fn empty_or_partial_frames_carry_no_speech() {
        assert!(!SpeechEvidence::measure(&[], RATE).is_speech());
        assert!(!SpeechEvidence::measure(&[1, 2, 3], RATE).is_speech());
    }

    #[test]
    fn constants_match_the_client_detector() {
        let client = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../vak-client-ui/src/voice-activity.ts"
        ))
        .expect("client detector source");
        for (name, value) in [
            ("ABSOLUTE_SPEECH_RMS", ABSOLUTE_SPEECH_RMS.to_string()),
            ("FLOOR_RATIO", FLOOR_RATIO.to_string()),
            ("MIN_VOICED_MS", MIN_VOICED_MS.to_string()),
            ("FLOOR_PERCENTILE", FLOOR_PERCENTILE.to_string()),
        ] {
            let needle = format!("export const {name} = {value};");
            assert!(client.contains(&needle), "client must declare `{needle}`");
        }
    }
}
