//! This Core's data catalog (plan M6): one per tenant, shared by every
//! Core in the process that reads the same data home, kept current by
//! catch-ups after each turn and on the scheduler tick.

use crate::{Core, CoreError};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use vak_catalog::{Audience, Catalog};

fn open_catalogs() -> &'static Mutex<HashMap<PathBuf, Weak<Catalog>>> {
    static CATALOGS: OnceLock<Mutex<HashMap<PathBuf, Weak<Catalog>>>> = OnceLock::new();
    CATALOGS.get_or_init(Default::default)
}

impl Core {
    /// The catalog of this Core's data home, opened once per process.
    pub fn catalog(&self) -> Result<Arc<Catalog>, CoreError> {
        let data = self.shared_scope().into_root();
        let path = vak_config::paths::tenant_home_at(&data, vak_config::paths::LOCAL_TENANT)
            .join(vak_catalog::DIR_NAME)
            .join(vak_catalog::FILE_NAME);
        let mut open = open_catalogs()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(catalog) = open.get(&path).and_then(Weak::upgrade) {
            return Ok(catalog);
        }
        let catalog = Arc::new(Catalog::open(&path, &data)?);
        open.retain(|_, held| held.strong_count() > 0);
        open.insert(path, Arc::downgrade(&catalog));
        Ok(catalog)
    }

    /// What this Core may read through the catalog: its Agent's things in
    /// its conversation audience when it serves one, everything otherwise,
    /// and never a session in the trash (invariant 37) or an intake item
    /// held back from the Agent (plan M6.5).
    pub fn catalog_audience(&self) -> Audience {
        Audience {
            agents: self
                .agent_identity()
                .map(|agent| vec![vak_session::trace::local::agent(&agent.id).to_string()]),
            audience: self
                .conversation_context()
                .map(|context| context.audience_id.clone()),
            exclude_sessions: crate::trash::trashed(&self.shared_scope()),
            held: false,
        }
    }

    /// Takes what the session ledger at `dir` appended, in the background.
    /// A missed catch-up costs latency only: the next one takes it.
    pub(crate) fn queue_catalog(&self, dir: &Path) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        {
            let mut pending = self
                .inner
                .catalog_pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !pending.insert(dir.to_path_buf()) {
                return;
            }
        }
        let core = self.clone();
        let dir = dir.to_path_buf();
        runtime.spawn_blocking(move || {
            let caught = core
                .catalog()
                .and_then(|catalog| Ok(catalog.catch_up_session(&dir)?));
            if let Err(error) = caught {
                tracing::warn!(
                    kind = "catalog",
                    error_kind = %vak_telemetry::error_kind(&error),
                    "a session was not taken into the catalog"
                );
            }
            core.inner
                .catalog_pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&dir);
        });
    }
}
