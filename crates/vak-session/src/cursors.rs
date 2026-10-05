//! Cursors (plan M4.7, docs/design/73-data-architecture-and-lifecycle.md
//! §8): a durable position in an external stream, a ref
//! `cur/<owner>/<stream>` moved by CAS under the writer epoch. An owner (a
//! bot's poller) holds its cursors through `cur/<owner>/holder`: a second
//! poller of the same owner cannot take it while the holder is alive, and a
//! holder that lost it (its liveness lapsed and another took over) can no
//! longer move a position, so it stops. A position that cannot be resumed
//! (expired at the provider, or older than the resume bound) is resynced to
//! a newer one, and the skipped range is a `gap` row in the `cursors/`
//! chain: never refetched in full, never lost silently.

use crate::chain::RecordChain;
use crate::ids::{PrincipalId, ProcessId};
use crate::objects::ObjectRef;
use crate::trace::TraceKey;
use crate::types::SessionError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What a stream's cursor ref holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub position: String,
    /// Items read past the position and not yet handled, as an object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backlog: Option<ObjectRef>,
    /// The position it was resynced from, when the last move was a resync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resynced_from: Option<String>,
    pub at: DateTime<Utc>,
}

/// What an owner's holder ref holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Holder {
    holder: ProcessId,
}

/// A range of a stream a cursor skipped when it resynced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CursorGap {
    pub owner: String,
    pub stream: String,
    pub at: DateTime<Utc>,
    /// The position that could not be resumed; `None` if there was none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The position it resynced to.
    pub to: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<PrincipalId>,
}

crate::impl_traced!(CursorGap, "cursor_gap");

/// Whether this process holds an owner's cursors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    Held,
    /// Another live process holds them.
    Elsewhere(ProcessId),
}

/// The cursors of one data home.
#[derive(Debug, Clone)]
pub struct Cursors {
    chain: RecordChain,
    tenant_home: PathBuf,
    holder: ProcessId,
}

fn holder_ref(owner: &str) -> String {
    format!("cur/{owner}/holder")
}

fn cursor_ref(owner: &str, stream: &str) -> String {
    format!("cur/{owner}/{stream}")
}

fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8], what: &str) -> Result<T, SessionError> {
    serde_json::from_slice(bytes).map_err(|error| SessionError::Objects(format!("{what}: {error}")))
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, SessionError> {
    serde_json::to_vec(value).map_err(|error| SessionError::Objects(error.to_string()))
}

impl Cursors {
    /// The gap rows in `dir` (`SharedScope::cursors`), with cursor refs in
    /// the tenant at `tenant_home`.
    pub fn at(dir: impl Into<PathBuf>, tenant_home: impl Into<PathBuf>) -> Self {
        Self {
            chain: RecordChain::at(dir),
            tenant_home: tenant_home.into(),
            holder: crate::fence::process(),
        }
    }

    /// These cursors, acting as the process `holder`, for tests and tools
    /// that stand in for another process; its liveness is renewed once, when
    /// it takes hold, and then lapses.
    pub fn as_process(mut self, holder: ProcessId) -> Self {
        self.holder = holder;
        self
    }

    pub fn path(&self) -> &Path {
        self.chain.path()
    }

    fn renew(&self) -> Result<(), SessionError> {
        if self.holder == crate::fence::process() {
            crate::runs::keep_alive(&self.tenant_home)
        } else {
            crate::fence::renew_liveness_of(
                &self.tenant_home,
                &self.holder,
                Utc::now() + chrono::Duration::seconds(crate::runs::LIVENESS_HOLD_SECS),
            )
        }
    }

    fn get_ref(&self, name: &str) -> Result<Option<Vec<u8>>, SessionError> {
        let tenant = crate::objects::TenantObjects::for_tenant(&self.tenant_home)?;
        Ok(tenant
            .store()
            .get_ref(name)
            .map_err(crate::objects::objects_error)?
            .map(|value| value.target))
    }

    /// Takes hold of `owner`'s cursors for this process, unless another
    /// live process holds them. A holder whose liveness lapsed is taken
    /// over.
    pub fn hold(&self, owner: &str, now: DateTime<Utc>) -> Result<Hold, SessionError> {
        self.renew()?;
        let me = self.holder;
        let mut elsewhere = None;
        crate::fence::swap_ref(&self.tenant_home, &holder_ref(owner), |target| {
            elsewhere = None;
            if let Some(bytes) = target {
                let current: Holder = decode(bytes, "cursor holder")?;
                if current.holder == me {
                    return Ok(None);
                }
                if crate::fence::is_alive(&self.tenant_home, &current.holder, now)? {
                    elsewhere = Some(current.holder);
                    return Ok(None);
                }
            }
            encode(&Holder { holder: me }).map(Some)
        })?;
        Ok(elsewhere.map_or(Hold::Held, Hold::Elsewhere))
    }

    /// Whether this process still holds `owner`'s cursors.
    pub fn holds(&self, owner: &str) -> Result<bool, SessionError> {
        Ok(match self.get_ref(&holder_ref(owner))? {
            Some(bytes) => decode::<Holder>(&bytes, "cursor holder")?.holder == self.holder,
            None => false,
        })
    }

    /// The cursor of `stream`, if it has one.
    pub fn get(&self, owner: &str, stream: &str) -> Result<Option<Cursor>, SessionError> {
        self.get_ref(&cursor_ref(owner, stream))?
            .map(|bytes| decode(&bytes, "cursor"))
            .transpose()
    }

    /// Moves `stream`'s cursor to `position`, with `backlog`, if this
    /// process still holds `owner`. Returns whether it moved: `false` means
    /// another process holds the owner now, and this one must stop.
    pub fn advance(
        &self,
        owner: &str,
        stream: &str,
        position: &str,
        backlog: Option<ObjectRef>,
    ) -> Result<bool, SessionError> {
        self.write(owner, stream, position, backlog, None)
    }

    fn write(
        &self,
        owner: &str,
        stream: &str,
        position: &str,
        backlog: Option<ObjectRef>,
        resynced_from: Option<String>,
    ) -> Result<bool, SessionError> {
        if !self.holds(owner)? {
            return Ok(false);
        }
        let next = encode(&Cursor {
            position: position.to_owned(),
            backlog,
            resynced_from,
            at: Utc::now(),
        })?;
        crate::fence::swap_ref(&self.tenant_home, &cursor_ref(owner, stream), |_| {
            Ok(Some(next.clone()))
        })?;
        Ok(true)
    }

    /// Resyncs `stream` to `position` because the old one could not be
    /// resumed: records the skipped range as a gap row, then moves the
    /// cursor. Returns whether it moved, as [`Cursors::advance`] does.
    pub fn resync(
        &self,
        owner: &str,
        stream: &str,
        position: &str,
        reason: &str,
    ) -> Result<bool, SessionError> {
        if !self.holds(owner)? {
            return Ok(false);
        }
        let from = self.get(owner, stream)?.map(|cursor| cursor.position);
        self.chain.append(&CursorGap {
            owner: owner.to_owned(),
            stream: stream.to_owned(),
            at: Utc::now(),
            from: from.clone(),
            to: position.to_owned(),
            reason: reason.to_owned(),
            trace: None,
            actor: None,
        })?;
        self.write(owner, stream, position, None, from)
    }

    /// Every gap row, oldest first. A row that is not a gap is an error.
    pub fn gaps(&self) -> Result<Vec<CursorGap>, SessionError> {
        self.chain
            .read::<serde_json::Value>()
            .into_iter()
            .map(|row| {
                serde_json::from_value(row).map_err(|error| SessionError::Corrupt {
                    line: 0,
                    message: format!("cursor gap: {error}"),
                })
            })
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn home() -> (tempfile::TempDir, Cursors) {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().expect("tempdir");
        let cursors = Cursors::at(dir.path().join("cursors"), dir.path().join("tenant"));
        (dir, cursors)
    }

    #[test]
    fn a_held_cursor_advances_and_reads_back() {
        let (_dir, cursors) = home();
        assert_eq!(cursors.hold("bot/a", Utc::now()).unwrap(), Hold::Held);
        assert!(cursors.get("bot/a", "chan").unwrap().is_none());
        assert!(cursors.advance("bot/a", "chan", "42", None).unwrap());
        assert_eq!(
            cursors.get("bot/a", "chan").unwrap().unwrap().position,
            "42"
        );
    }

    #[test]
    fn second_poller_is_fenced() {
        let (_dir, cursors) = home();
        let first = cursors.clone().as_process(ProcessId::new());
        let second = cursors.clone().as_process(ProcessId::new());
        let now = Utc::now();
        assert_eq!(first.hold("bot/a", now).unwrap(), Hold::Held);
        assert!(first.advance("bot/a", "chan", "1", None).unwrap());
        // While the first is alive, the second cannot take hold or move it.
        assert!(matches!(
            second.hold("bot/a", now).unwrap(),
            Hold::Elsewhere(_)
        ));
        assert!(!second.advance("bot/a", "chan", "2", None).unwrap());
        assert_eq!(cursors.get("bot/a", "chan").unwrap().unwrap().position, "1");
        // Once the first lapses, the second takes over, and the first, if
        // it wakes, can no longer move the cursor.
        let later = now + chrono::Duration::seconds(crate::runs::LIVENESS_HOLD_SECS + 1);
        assert_eq!(second.hold("bot/a", later).unwrap(), Hold::Held);
        assert!(second.advance("bot/a", "chan", "3", None).unwrap());
        assert!(!first.advance("bot/a", "chan", "4", None).unwrap());
        assert_eq!(cursors.get("bot/a", "chan").unwrap().unwrap().position, "3");
    }

    #[test]
    fn cursor_resync_records_gap() {
        let (_dir, cursors) = home();
        cursors.hold("bot/a", Utc::now()).unwrap();
        cursors.advance("bot/a", "chan", "100", None).unwrap();
        assert!(
            cursors
                .resync("bot/a", "chan", "900", "older than the resume bound")
                .unwrap()
        );
        let cursor = cursors.get("bot/a", "chan").unwrap().unwrap();
        assert_eq!(cursor.position, "900");
        assert_eq!(cursor.resynced_from.as_deref(), Some("100"));
        let gaps = cursors.gaps().unwrap();
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].from.as_deref(), Some("100"));
        assert_eq!(gaps[0].to, "900");
        assert_eq!(gaps[0].stream, "chan");
    }
}
