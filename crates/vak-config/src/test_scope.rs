//! A test's isolated world: the process-shared private home plus a fresh
//! workspace directory of its own. Behind the `test-support` feature (and
//! always in this crate's own tests) so production builds never carry it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct TestScope {
    home: PathBuf,
    workspace: PathBuf,
}

impl TestScope {
    /// Isolates the data home (`paths::isolate_home_for_tests`) and creates
    /// an empty workspace directory unique to this scope.
    pub fn new() -> std::io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let home = crate::paths::isolate_home_for_tests();
        let workspace = home
            .join("scopes")
            .join(format!("ws-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&workspace)?;
        Ok(Self { home, workspace })
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn scopes_share_one_home_and_own_their_workspace() {
        let a = TestScope::new().unwrap();
        let b = TestScope::new().unwrap();
        assert_eq!(a.home(), b.home());
        assert_ne!(a.workspace(), b.workspace());
        assert!(a.workspace().is_dir());
        assert!(a.workspace().starts_with(a.home()));
    }
}
