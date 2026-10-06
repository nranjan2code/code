//! A record chain's projection kept as a Document (plan M6.3), so a turn
//! reads current state without replaying the chain. The Document holds
//! the state and the chain position it covers; a read folds whatever was
//! appended after that position (normally nothing, and never more than the
//! newest segments) and saves the result when it folded anything. A crash
//! between an append and a save costs one fold on the next read, never a
//! wrong answer, a save never moves the Document back past a position
//! another process already saved, and a chain found behind its rollup (a
//! cut unsynced tail) is rolled up again from its start.

use crate::tail::{self, Position};
use crate::types::SessionError;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

#[derive(serde::Serialize, serde::Deserialize)]
struct Stored<S> {
    position: Position,
    state: S,
}

/// A chain and the Document that rolls it up.
#[derive(Debug, Clone)]
pub struct Rollup {
    chain: PathBuf,
    document: PathBuf,
}

impl Rollup {
    pub fn new(chain: impl Into<PathBuf>, document: impl Into<PathBuf>) -> Self {
        Self {
            chain: chain.into(),
            document: document.into(),
        }
    }

    pub fn chain(&self) -> &Path {
        &self.chain
    }

    /// The state as of the chain's last row: the stored state, with every
    /// row appended since folded in by `fold` (which sees each row's bytes,
    /// oldest first). A Document that is missing or does not decode is
    /// rebuilt from the chain's start.
    pub fn read<S>(&self, mut fold: impl FnMut(&mut S, &[u8])) -> S
    where
        S: Default + Serialize + DeserializeOwned,
    {
        let stored = crate::documents::read(&self.document)
            .ok()
            .flatten()
            .and_then(|text| serde_json::from_str::<Stored<S>>(&text).ok());
        let (mut from, mut state) = stored.map_or_else(
            || (Position::default(), S::default()),
            |stored| (stored.position, stored.state),
        );
        // A chain behind the stored position lost rows the rollup counted
        // (an unsynced tail cut after a crash): rebuild rather than skip the
        // rows written in their place.
        if tail::head(&self.chain) < from {
            from = Position::default();
            state = S::default();
        }
        let mut folded = false;
        let to = tail::tail(&self.chain, from, |_, bytes| {
            fold(&mut state, bytes);
            folded = true;
            true
        });
        if folded && let Err(error) = self.save(to, &state) {
            tracing::warn!(
                kind = "rollup",
                error_kind = %vak_telemetry::error_kind(&error),
                "a rollup was not saved; the next read folds the same rows"
            );
        }
        state
    }

    /// Saves `state` as covering the chain up to `position`, unless another
    /// writer already saved a later position.
    fn save<S: Serialize + DeserializeOwned>(
        &self,
        position: Position,
        state: &S,
    ) -> Result<(), SessionError> {
        let text = serde_json::to_string(&Stored { position, state })
            .map_err(|error| SessionError::Objects(error.to_string()))?;
        crate::documents::update(&self.document, |current| {
            let later = current
                .and_then(|text| serde_json::from_str::<Stored<serde_json::Value>>(text).ok())
                .is_some_and(|stored| stored.position >= position);
            Ok(if later {
                None
            } else {
                Some((text.clone(), ()))
            })
        })
        .map(|_| ())
        .map_err(SessionError::Objects)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[derive(Default, serde::Serialize, serde::Deserialize)]
    struct Sum {
        total: u64,
        rows: u64,
    }

    fn fold(sum: &mut Sum, bytes: &[u8]) {
        let row: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        sum.total += row["n"].as_u64().unwrap();
        sum.rows += 1;
    }

    #[test]
    fn reads_fold_only_what_was_appended_since_and_never_reread_old_segments() {
        vak_config::paths::isolate_home_for_tests();
        let home = vak_config::paths::data_home().join("rollup-test");
        let chain = crate::chain::RecordChain::at(home.join("rows"));
        let rollup = Rollup::new(chain.path(), home.join("rows-rollup"));
        for n in 0..300u64 {
            // Incompressible rows, so they fill and seal segments.
            let pad: String = (0..64).map(|_| uuid::Uuid::new_v4().to_string()).collect();
            chain
                .append(&serde_json::json!({ "n": n, "pad": pad }))
                .unwrap();
        }
        let first: Sum = rollup.read(fold);
        assert_eq!((first.rows, first.total), (300, (0..300).sum()));
        // Every sealed segment becomes unreadable: a read that went back to
        // them would fail; this one never does.
        let mut sealed = 0;
        for entry in std::fs::read_dir(chain.path()).unwrap().flatten() {
            if entry.file_name().to_string_lossy().ends_with(".sealed") {
                std::fs::write(entry.path(), b"gone").unwrap();
                sealed += 1;
            }
        }
        assert!(sealed > 0);
        chain.append(&serde_json::json!({ "n": 1000 })).unwrap();
        let next: Sum = rollup.read(fold);
        assert_eq!((next.rows, next.total), (301, (0..300).sum::<u64>() + 1000));
    }
}
