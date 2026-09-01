//! Chat-surface transport adapters: long-polling or webhook clients that
//! bridge an external surface into `POST /gateway/inbound` and deliver the
//! reply back.
//!
//! Deliberately distinct from `vak-delivery`'s same-named modules, which own
//! the *markup projection* for each surface (Telegram HTML, Slack mrkdwn,
//! Discord markdown). Transport lives here; formatting lives there. Both sets
//! previously sat at `src/telegram.rs` in their respective crates, which read
//! as duplication until you opened both.

pub mod discord;
pub mod slack;
pub mod telegram;
