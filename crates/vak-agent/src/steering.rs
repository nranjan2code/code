use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use vak_llm::Message;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainMode {
    OneAtATime,
    All,
}

#[derive(Debug, Default)]
struct Queues {
    steering: VecDeque<Message>,
    follow_up: VecDeque<Message>,
}

/// Two queues with two polling sites: steering interrupts the current run,
/// follow-up continues after a natural stop. Interior-mutable so the UI can
/// push while an agent run holds only a shared reference. Entries carry full
/// user messages (text + image blocks) so queued input is never degraded —
/// model-visible content must match what the sender supplied (invariant 1).
#[derive(Debug, Default)]
pub struct SteeringQueues {
    q: Mutex<Queues>,
}

impl SteeringQueues {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_steering(&self, text: impl Into<String>) {
        self.push_steering_message(Message::user_text(text));
    }

    pub fn push_steering_message(&self, message: Message) {
        self.lock().steering.push_back(message);
    }

    pub fn push_follow_up(&self, text: impl Into<String>) {
        self.lock().follow_up.push_back(Message::user_text(text));
    }

    pub fn drain(&self, mode: DrainMode) -> Vec<Message> {
        let take = match mode {
            DrainMode::OneAtATime => 1,
            DrainMode::All => usize::MAX,
        };
        let mut q = self.lock();
        let mut out = Vec::new();
        while out.len() < take {
            match q.steering.pop_front() {
                Some(m) => out.push(m),
                None => break,
            }
        }
        out
    }

    pub fn take_follow_up(&self) -> Option<Message> {
        self.lock().follow_up.pop_front()
    }

    pub fn has_pending(&self) -> bool {
        let q = self.lock();
        !q.steering.is_empty() || !q.follow_up.is_empty()
    }

    /// Merge drained messages into one prompt turn. Same-role block
    /// concatenation keeps text AND image blocks; the merged message is
    /// exactly what gets logged and dispatched.
    pub fn merge_prompt(messages: Vec<Message>) -> Option<Message> {
        if messages.is_empty() {
            return None;
        }
        if messages.len() == 1 {
            return messages.into_iter().next();
        }
        let mut content = Vec::new();
        for m in messages {
            content.extend(m.content);
        }
        Some(Message {
            role: vak_llm::Role::User,
            content,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Queues> {
        self.q.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use vak_llm::{ContentBlock, Role};

    #[test]
    fn steering_round_trips_images_through_merge() {
        let q = SteeringQueues::new();
        q.push_steering("plain");
        let with_image = Message {
            role: Role::User,
            content: vec![
                ContentBlock::text("look"),
                ContentBlock::image_base64("image/png", "QUJD"),
            ],
        };
        q.push_steering_message(with_image);

        let drained = q.drain(DrainMode::All);
        assert_eq!(drained.len(), 2);
        let merged = SteeringQueues::merge_prompt(drained).unwrap();
        assert_eq!(merged.role, Role::User);
        assert_eq!(merged.content.len(), 3, "text + image + text survive");
    }

    #[test]
    fn empty_drain_merges_to_none() {
        assert!(SteeringQueues::merge_prompt(Vec::new()).is_none());
    }
}
