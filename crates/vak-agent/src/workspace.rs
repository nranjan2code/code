//! Workspace-delta seam (docs/design/27 Phase H MEA): the auditor judges
//! environment facts, not just transcript claims. Core implements this by
//! diffing the run-start checkpoint against the live tree.

pub trait WorkspaceDelta: Send + Sync {
    /// Bounded human-readable summary; Err = unavailable (auditor is told
    /// the delta is UNKNOWN rather than shown an empty one).
    fn summary(&self) -> Result<String, String>;
}
