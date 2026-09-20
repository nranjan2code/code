//! vak-context: the context engine (docs/design/68-context-engine.md).
//!
//! Everything that decides *what one request carries* for the model in
//! front of it, as pure functions over the session ledger:
//!
//! - [`capacity`] — `CapacityProfile`: the measured capacity of one model on
//!   one provider (window, usable instruction horizon, tokens per char,
//!   prefill rate, cache behaviour), probed at bind time and revised from
//!   every receipt. Nothing in a request is sized from a declared number.
//! - [`planner`] — `WorkingSetPlanner`: which closed turns ride along at
//!   `Full`, `Card` or `Packet` fidelity for this request, chosen against
//!   the measured budget by `max(recency, relevance, anaphora)`. A turn is
//!   never split; nothing model-visible is cut by a character count.
//! - [`assemble`] — the request assembler: the byte-stable prefix and its
//!   digest, the per-turn tail attached to the last user message, cache
//!   breakpoints per §10, char accounting that feeds the profile, and the
//!   summariser request behind a compaction packet.
//!
//! The ledger itself — `TurnIndex`, `TurnCard`, `derive_with_plan`, the
//! range-keyed packets and `recall` — lives in `vak-session`: that is the
//! one rich original every projection is computed from, and this crate
//! only reads it. The agent loop (`vak-agent`) and the host (`vak-core`)
//! own the I/O: probing, the summariser call, and writing what came back.
//! Nothing here performs network or disk I/O.

pub mod assemble;
pub mod capacity;
pub mod planner;

pub use assemble::TailInput;
pub use capacity::{CapacityProfile, ProfileKey};
pub use planner::{PlanInput, plan, plan_for_session};
pub use vak_session::{Fidelity, WorkingSetPlan};
