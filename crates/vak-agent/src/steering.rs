use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainMode {
    OneAtATime,
    All,
}

/// Two queues with two polling sites: steering interrupts the current run,
/// follow-up continues after a natural stop.
#[derive(Debug, Default)]
pub struct SteeringQueues {
    steering: VecDeque<String>,
    follow_up: VecDeque<String>,
}

impl SteeringQueues {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_steering(&mut self, text: impl Into<String>) {
        self.steering.push_back(text.into());
    }

    pub fn push_follow_up(&mut self, text: impl Into<String>) {
        self.follow_up.push_back(text.into());
    }

    pub fn drain(&mut self, mode: DrainMode) -> Vec<String> {
        let take = match mode {
            DrainMode::OneAtATime => 1,
            DrainMode::All => usize::MAX,
        };
        let mut out = Vec::new();
        while out.len() < take {
            match self.steering.pop_front() {
                Some(t) => out.push(t),
                None => break,
            }
        }
        out
    }

    pub fn take_follow_up(&mut self) -> Option<String> {
        self.follow_up.pop_front()
    }

    pub fn has_pending(&self) -> bool {
        !self.steering.is_empty() || !self.follow_up.is_empty()
    }
}
