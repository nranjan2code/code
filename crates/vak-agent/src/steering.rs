use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainMode {
    OneAtATime,
    All,
}

#[derive(Debug, Default)]
struct Queues {
    steering: VecDeque<String>,
    follow_up: VecDeque<String>,
}

/// Two queues with two polling sites: steering interrupts the current run,
/// follow-up continues after a natural stop. Interior-mutable so the UI can
/// push while an agent run holds only a shared reference.
#[derive(Debug, Default)]
pub struct SteeringQueues {
    q: Mutex<Queues>,
}

impl SteeringQueues {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_steering(&self, text: impl Into<String>) {
        self.lock().steering.push_back(text.into());
    }

    pub fn push_follow_up(&self, text: impl Into<String>) {
        self.lock().follow_up.push_back(text.into());
    }

    pub fn drain(&self, mode: DrainMode) -> Vec<String> {
        let take = match mode {
            DrainMode::OneAtATime => 1,
            DrainMode::All => usize::MAX,
        };
        let mut q = self.lock();
        let mut out = Vec::new();
        while out.len() < take {
            match q.steering.pop_front() {
                Some(t) => out.push(t),
                None => break,
            }
        }
        out
    }

    pub fn take_follow_up(&self) -> Option<String> {
        self.lock().follow_up.pop_front()
    }

    pub fn has_pending(&self) -> bool {
        let q = self.lock();
        !q.steering.is_empty() || !q.follow_up.is_empty()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Queues> {
        self.q.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
