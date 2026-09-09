use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    Listening,
    Thinking,
    Speaking,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    SpeechStarted {
        utterance_id: String,
    },
    TranscriptCommitted {
        utterance_id: String,
        text: String,
    },
    PlaybackInterrupted {
        utterance_id: String,
        emitted_ms: u64,
    },
    RunCancelled,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLimits {
    pub max_duration: Duration,
    pub max_concurrent: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct VoiceRuntimeConfig {
    pub enabled: bool,
    pub max_session_secs: u64,
    pub max_concurrent: usize,
    pub max_audio_bytes: usize,
}

impl Default for VoiceRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_session_secs: 900,
            max_concurrent: 2,
            max_audio_bytes: 16 * 1024 * 1024,
        }
    }
}

impl VoiceRuntimeConfig {
    pub fn validate(&self) -> Result<(), crate::VoiceError> {
        SessionLimits {
            max_duration: Duration::from_secs(self.max_session_secs),
            max_concurrent: self.max_concurrent,
        }
        .validate()?;
        if self.max_audio_bytes == 0 || self.max_audio_bytes > 256 * 1024 * 1024 {
            return Err(crate::VoiceError::InvalidRequest(
                "voice audio limit must be between 1 byte and 256 MiB".into(),
            ));
        }
        Ok(())
    }
}

impl SessionLimits {
    pub fn validate(self) -> Result<(), crate::VoiceError> {
        if self.max_duration.is_zero() {
            return Err(crate::VoiceError::InvalidRequest(
                "voice session duration must be positive".into(),
            ));
        }
        if self.max_concurrent == 0 {
            return Err(crate::VoiceError::InvalidRequest(
                "voice concurrency must be positive".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SessionPool {
    active: Arc<AtomicUsize>,
    max: usize,
}

impl SessionPool {
    pub fn new(max: usize) -> Result<Self, crate::VoiceError> {
        if max == 0 {
            return Err(crate::VoiceError::InvalidRequest(
                "voice concurrency must be positive".into(),
            ));
        }
        Ok(Self {
            active: Arc::new(AtomicUsize::new(0)),
            max,
        })
    }
    pub fn try_acquire(&self) -> Option<SessionPermit> {
        self.active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.max).then_some(n + 1)
            })
            .ok()
            .map(|_| SessionPermit {
                active: Arc::clone(&self.active),
            })
    }
    pub fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }
    pub fn max(&self) -> usize {
        self.max
    }
}

#[derive(Debug)]
pub struct SessionPermit {
    active: Arc<AtomicUsize>,
}
impl Drop for SessionPermit {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Server-side lifecycle guard. Playback interruption and agent cancellation
/// are separate transitions so a user can stop audio while work continues.
#[derive(Debug)]
pub struct VoiceSession {
    started: Instant,
    max_duration: Duration,
    phase: SessionPhase,
    active_utterance: Option<String>,
}

impl VoiceSession {
    pub fn try_new(limits: SessionLimits) -> Result<Self, crate::VoiceError> {
        limits.validate()?;
        Ok(Self::new(limits.max_duration))
    }
    pub fn new(max_duration: Duration) -> Self {
        Self {
            started: Instant::now(),
            max_duration,
            phase: SessionPhase::Listening,
            active_utterance: None,
        }
    }
    pub fn phase(&self) -> SessionPhase {
        self.phase
    }
    pub fn expired(&self) -> bool {
        self.started.elapsed() >= self.max_duration
    }
    pub fn remaining(&self) -> Duration {
        self.max_duration.saturating_sub(self.started.elapsed())
    }
    pub fn start_speech(&mut self, id: impl Into<String>) -> Option<SessionEvent> {
        if self.expired() {
            self.phase = SessionPhase::Closed;
            return Some(SessionEvent::Expired);
        }
        let id = id.into();
        self.active_utterance = Some(id.clone());
        self.phase = SessionPhase::Listening;
        Some(SessionEvent::SpeechStarted { utterance_id: id })
    }
    pub fn commit_transcript(&mut self, id: &str, text: impl Into<String>) -> Option<SessionEvent> {
        if self.phase == SessionPhase::Closed || self.active_utterance.as_deref() != Some(id) {
            return None;
        }
        let text = text.into();
        if text.trim().is_empty() {
            return None;
        }
        self.phase = SessionPhase::Thinking;
        Some(SessionEvent::TranscriptCommitted {
            utterance_id: id.into(),
            text,
        })
    }
    pub fn begin_playback(&mut self) -> bool {
        if self.phase == SessionPhase::Closed || self.expired() {
            self.phase = SessionPhase::Closed;
            return false;
        }
        self.phase = SessionPhase::Speaking;
        true
    }
    pub fn interrupt_playback(&mut self, emitted_ms: u64) -> Option<SessionEvent> {
        if self.phase != SessionPhase::Speaking {
            return None;
        }
        self.phase = SessionPhase::Listening;
        Some(SessionEvent::PlaybackInterrupted {
            utterance_id: self.active_utterance.clone().unwrap_or_default(),
            emitted_ms,
        })
    }
    pub fn cancel_run(&mut self) -> Option<SessionEvent> {
        if self.phase == SessionPhase::Closed {
            return None;
        }
        // Cancellation is terminal for the current run. The transport may
        // remain open so the client can start a fresh utterance, but no stale
        // transcript or playback transition can be accepted for this one.
        self.active_utterance = None;
        self.phase = SessionPhase::Listening;
        Some(SessionEvent::RunCancelled)
    }
    pub fn close(&mut self) {
        self.phase = SessionPhase::Closed;
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn playback_interrupt_does_not_close_session_or_cancel_run() {
        let mut s = VoiceSession::new(Duration::from_secs(60));
        s.start_speech("u1");
        s.commit_transcript("u1", "inspect tests");
        assert!(s.begin_playback());
        assert_eq!(
            s.interrupt_playback(240),
            Some(SessionEvent::PlaybackInterrupted {
                utterance_id: "u1".into(),
                emitted_ms: 240
            })
        );
        assert_eq!(s.phase(), SessionPhase::Listening);
        assert_eq!(s.cancel_run(), Some(SessionEvent::RunCancelled));
    }
    #[test]
    fn wrong_utterance_cannot_commit() {
        let mut s = VoiceSession::new(Duration::from_secs(60));
        s.start_speech("u1");
        assert!(s.commit_transcript("u2", "ignored").is_none());
    }

    #[test]
    fn cancelling_run_discards_stale_utterance() {
        let mut session = VoiceSession::new(Duration::from_secs(10));
        session.start_speech("u1");
        assert!(session.cancel_run().is_some());
        assert!(session.commit_transcript("u1", "stale").is_none());
        assert!(session.start_speech("u2").is_some());
    }

    #[test]
    fn limits_reject_unbounded_configuration() {
        assert!(
            SessionLimits {
                max_duration: Duration::ZERO,
                max_concurrent: 1
            }
            .validate()
            .is_err()
        );
        assert!(
            VoiceRuntimeConfig {
                max_audio_bytes: 256 * 1024 * 1024 + 1,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            SessionLimits {
                max_duration: Duration::from_secs(1),
                max_concurrent: 0
            }
            .validate()
            .is_err()
        );
        assert!(
            SessionLimits {
                max_duration: Duration::from_secs(1),
                max_concurrent: 1
            }
            .validate()
            .is_ok()
        );
        assert!(
            VoiceSession::try_new(SessionLimits {
                max_duration: Duration::ZERO,
                max_concurrent: 1
            })
            .is_err()
        );
        assert!(
            VoiceSession::try_new(SessionLimits {
                max_duration: Duration::from_secs(1),
                max_concurrent: 1
            })
            .is_ok()
        );
        let pool = SessionPool::new(1).unwrap();
        let permit = pool.try_acquire().unwrap();
        assert!(pool.try_acquire().is_none());
        drop(permit);
        assert!(pool.try_acquire().is_some());
    }
}
