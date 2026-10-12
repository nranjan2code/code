//! The crate's integration tests, one binary (each file a module).

#[path = "../support/mod.rs"]
mod support;

mod admission_trace;
mod agent_identity_per_turn;
mod artifact_versions;
mod capability_lifecycle;
mod checkpoint_label;
mod claims;
mod derived_provenance;
mod docker_sandbox;
mod grants;
mod intake_retrieval;
mod intent_projection;
mod intent_wiring;
mod learning_loop;
mod ledger_objects;
mod lifecycle_quota;
mod mcp_env_interp;
mod permissions_local;
mod reflection;
mod reflection_seam;
mod session_search_tool;
mod side_ledger_trace;
mod state_registry;
mod turn_path_reads_flat;
mod worktree;
