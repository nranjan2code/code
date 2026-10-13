//! Receipts to verdicts (docs/design/88-control-plane.md §7).
//!
//! The agent loop gathers what was observed (a call's schema check, a
//! permission decision, a typed fault, the cards and files a turn produced)
//! and the functions here say what follows. Each is pure: no lock, no I/O,
//! no session handle, nothing read from the wording of a request, an answer
//! or an error.

pub mod call;
pub mod measure;
pub mod outcome;
pub mod plan;
