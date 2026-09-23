//! The capability subsystem: one registry, one reconcile loop, one
//! projection, for all five kinds (tool, skill, MCP server, hook, command).
//!
//! See `docs/design/41-capability-registry.md`. The short version:
//!
//! * A capability **declares what it serves** (`domain`), so the harness
//!   matches instead of holding a table of built-in names. Adding an
//!   integration never edits the harness.
//! * Declaration is **offline**. Nothing here spawns or probes: an MCP
//!   server's catalog and last failure are what the on-demand pool observed,
//!   declared as data (`resolution`, `vak_mcp::McpManager`).
//! * The registry publishes **immutable versioned snapshots**
//!   (`snapshot`), and a **turn** binds one for its whole duration. That is
//!   what lets a three-week-old session pick up a skill added today with no
//!   restart and no session rotation.
//! * The loop (`registry`) is **level-triggered**: hints make it run sooner,
//!   the ticker makes it run anyway, and every pass is idempotent.
//! * Everything visible derives from one `report`, so the model, `doctor`
//!   and the admin console cannot disagree.

pub mod domain;
pub mod provider;
pub mod registry;
pub mod report;
pub mod resolution;
pub mod snapshot;
pub mod surface;
pub mod turn;

pub use domain::{Domain, Serves};
pub use registry::{
    CapabilityProvider, CapabilityRegistry, DEBOUNCE, Declaration, Hint, RECONCILE_INTERVAL,
    ReconcileStatus,
};
pub use report::{CapabilityReport, CapabilityRow, standing_section};
pub use resolution::Resolution;
pub use snapshot::{
    Binding, Capability, CapabilityDelta, CapabilityId, CapabilitySet, Epoch, Origin,
};
pub use surface::{ToolSurface, build_tool_surface, tool_catalogue};
pub use turn::{TurnCapabilities, TurnProbe};

/// The MCP inventory: a list of (server name, discovered tools).
pub type McpInventory = Vec<(String, Vec<vak_mcp::McpToolInfo>)>;
