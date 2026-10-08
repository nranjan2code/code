//! Intake (plan M6.5, docs/design/76-intake-and-knowledge.md): sources an
//! Agent follows, polled by their trigger, and what each poll took.
//!
//! A source is a Document `sources/<src>` naming its Agent, its connector
//! (`vak_intake::Connector`, a closed set) and the trigger that polls it.
//! A poll is a run: it holds the source's cursor
//! (`cur/agent/<agent>/intake/<src>`), fetches with webfetch's guard, has
//! the worker parse the bytes, and takes each item it has not seen: the
//! item as a tenant object, and a `taken` row in the `intake/` chain with
//! the run's trace key and what detection said. Detection labels and never
//! drops; a person releases or holds an item with a row of their own.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use vak_config::scope::SharedScope;
use vak_session::chain::RecordChain;
use vak_session::ids::{PrincipalId, SourceId, TriggerId};
use vak_session::objects::{ObjectRef, Objects, TenantObjects};
use vak_session::trace::TraceKey;

pub use vak_intake::{Connector, Detection, Disposition, Item};

/// How much a source's items are trusted as evidence; it never changes
/// what detection holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    Low,
    #[default]
    Medium,
    High,
}

/// A source an Agent follows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub id: SourceId,
    pub name: String,
    /// The Agent whose items these are (`AgentDefinition::id`).
    pub agent: String,
    pub connector: Connector,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub trust: Trust,
    /// The trigger that polls it (a `source_poll` automation).
    pub trigger: TriggerId,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<PrincipalId>,
}

#[derive(Debug, thiserror::Error)]
pub enum IntakeError {
    #[error("{0}")]
    Invalid(String),
    #[error("no source {0}")]
    NotFound(String),
    #[error("intake store: {0}")]
    Store(String),
    #[error("source {0} cannot be read: {1}")]
    Corrupt(String, String),
    /// Another live process holds the source's cursor.
    #[error("another process is polling this source")]
    Busy,
    #[error("the fetch failed: {0}")]
    Fetch(String),
    #[error("what the source returned could not be read: {0}")]
    Parse(String),
}

impl Source {
    pub fn validate(&self) -> Result<(), IntakeError> {
        if self.name.trim().is_empty() || self.name.chars().count() > 120 {
            return Err(IntakeError::Invalid(
                "a source's name is 1 to 120 characters".into(),
            ));
        }
        if self.agent.trim().is_empty() {
            return Err(IntakeError::Invalid("a source names its Agent".into()));
        }
        if self.tags.len() > 20
            || self
                .tags
                .iter()
                .any(|tag| tag.trim().is_empty() || tag.len() > 40)
        {
            return Err(IntakeError::Invalid(
                "a source has at most 20 tags of 1 to 40 characters".into(),
            ));
        }
        self.connector
            .validate()
            .map_err(|error| IntakeError::Invalid(error.to_string()))
    }

    /// The cursor owner a poll holds.
    pub fn cursor_owner(&self) -> String {
        format!("agent/{}/intake", self.agent)
    }

    /// The grant scope of its items' objects.
    pub fn object_scope(&self) -> String {
        object_scope(&self.id)
    }
}

/// The built-in source a file arrives through when a person drops it or a
/// channel attaches it (push intake, doc 76 §1): one per Agent, never a
/// Document, never polled.
pub fn push_source(agent: &str) -> SourceId {
    SourceId::derived(&format!("push/{agent}"))
}

/// The grant scope of a source's item objects.
pub fn object_scope(source: &SourceId) -> String {
    format!("intake:{source}")
}

/// An item's catalog id: stable for one key of one source, so a repeated
/// poll takes the same item, never a second one.
pub fn item_id(source: &SourceId, item: &Item) -> String {
    format!("itm:{source}:{}", &item.key_digest()[..32])
}

fn source_path(shared: &SharedScope, id: &str) -> PathBuf {
    shared.sources().join(id)
}

fn decode(name: &str, text: &str) -> Result<Source, IntakeError> {
    serde_json::from_str(text).map_err(|error| IntakeError::Corrupt(name.into(), error.to_string()))
}

/// Every source, oldest first. One that cannot be read is an error.
pub fn list(shared: &SharedScope) -> Result<Vec<Source>, IntakeError> {
    let mut sources = Vec::new();
    for path in vak_session::documents::under(&shared.sources()) {
        let name = path.display().to_string();
        if let Some(text) = vak_session::documents::read(&path).map_err(IntakeError::Store)? {
            sources.push(decode(&name, &text)?);
        }
    }
    sources.sort_by_key(|source| source.created_at);
    Ok(sources)
}

pub fn get(shared: &SharedScope, id: &str) -> Result<Option<Source>, IntakeError> {
    match vak_session::documents::read(&source_path(shared, id)).map_err(IntakeError::Store)? {
        Some(text) => decode(id, &text).map(Some),
        None => Ok(None),
    }
}

/// Saves a new source; fails if one with its id exists.
pub fn create(shared: &SharedScope, source: &Source) -> Result<(), IntakeError> {
    source.validate()?;
    let text =
        serde_json::to_string(source).map_err(|error| IntakeError::Store(error.to_string()))?;
    vak_session::documents::create(&source_path(shared, &source.id.to_string()), &text)
        .map_err(IntakeError::Store)
}

/// Applies `change` and saves the result as a new version. A source's id,
/// Agent, trigger and creation never change.
pub fn update(
    shared: &SharedScope,
    id: &str,
    mut change: impl FnMut(&mut Source) -> Result<(), IntakeError>,
) -> Result<Source, IntakeError> {
    let mut failure: Option<IntakeError> = None;
    let saved = vak_session::documents::update(&source_path(shared, id), |current| {
        let Some(text) = current else {
            failure = Some(IntakeError::NotFound(id.into()));
            return Ok(None);
        };
        let mut source = match decode(id, text) {
            Ok(source) => source,
            Err(error) => {
                failure = Some(error);
                return Ok(None);
            }
        };
        let fixed = (
            source.id,
            source.agent.clone(),
            source.trigger,
            source.created_at,
        );
        if let Err(error) = change(&mut source).and_then(|()| source.validate()) {
            failure = Some(error);
            return Ok(None);
        }
        if (
            source.id,
            source.agent.clone(),
            source.trigger,
            source.created_at,
        ) != fixed
        {
            failure = Some(IntakeError::Invalid(
                "a source's id, Agent, trigger and creation time never change".into(),
            ));
            return Ok(None);
        }
        let text = serde_json::to_string(&source).map_err(|error| error.to_string())?;
        Ok(Some((text, source)))
    })
    .map_err(IntakeError::Store)?;
    match (saved, failure) {
        (Some(source), _) => Ok(source),
        (None, Some(error)) => Err(error),
        (None, None) => Err(IntakeError::NotFound(id.into())),
    }
}

/// Forgets a source; its items and their rows stay until erasure (M7a).
pub fn delete(shared: &SharedScope, id: &str) -> Result<bool, IntakeError> {
    vak_session::documents::forget(&source_path(shared, id)).map_err(IntakeError::Store)
}

/// One row of the `intake/` chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntakeEvent {
    /// The item's catalog id ([`item_id`]).
    pub item: String,
    pub at: DateTime<Utc>,
    /// The poll's run key, on the row that takes the item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceKey>,
    /// Who caused this step: the poll's actor, or the person who released
    /// or held the item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<PrincipalId>,
    #[serde(flatten)]
    pub step: IntakeStep,
}

vak_session::impl_traced!(IntakeEvent, "intake_event");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum IntakeStep {
    /// A poll took the item: its body is `object`, and detection said
    /// `disposition` with its labels and evidence.
    Taken {
        source: SourceId,
        agent: String,
        key: String,
        object: ObjectRef,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        link: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        published: Option<DateTime<Utc>>,
        disposition: Disposition,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        labels: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        evidence: Vec<String>,
    },
    /// A person let a held item reach the Agent.
    Released,
    /// A person held an item back from the Agent.
    Quarantined {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

/// The intake chain of a data home and the tenant its objects live in.
#[derive(Debug, Clone)]
pub struct Intake {
    chain: RecordChain,
    tenant_home: PathBuf,
}

impl Intake {
    pub fn at(shared: &SharedScope, tenant_home: impl Into<PathBuf>) -> Self {
        Self {
            chain: RecordChain::at(shared.intake()),
            tenant_home: tenant_home.into(),
        }
    }

    pub fn path(&self) -> &Path {
        self.chain.path()
    }

    /// Takes each of `items` not in `seen`: its body as a tenant object,
    /// then one `taken` row. Returns the rows written. `seen` gains every
    /// key taken.
    pub fn take(
        &self,
        source: &Source,
        trace: &TraceKey,
        items: &[Item],
        seen: &mut Vec<String>,
    ) -> Result<Vec<IntakeEvent>, IntakeError> {
        let objects = TenantObjects::for_tenant(&self.tenant_home)
            .map_err(|error| IntakeError::Store(error.to_string()))?;
        let scope = source.object_scope();
        let mut rows = Vec::new();
        for item in items {
            if seen.contains(&item.key) {
                continue;
            }
            let body =
                serde_json::to_vec(item).map_err(|error| IntakeError::Store(error.to_string()))?;
            let object = objects
                .put(&body, &scope)
                .map_err(|error| IntakeError::Store(error.to_string()))?;
            let detection = vak_intake::detect(item);
            rows.push(IntakeEvent {
                item: item_id(&source.id, item),
                at: Utc::now(),
                trace: Some(trace.clone()),
                actor: trace.actor,
                step: IntakeStep::Taken {
                    source: source.id,
                    agent: source.agent.clone(),
                    key: item.key.clone(),
                    object,
                    title: item.title.clone(),
                    link: item.link.clone(),
                    published: item.published,
                    disposition: detection.disposition,
                    labels: detection.labels,
                    evidence: detection.evidence,
                },
            });
            seen.push(item.key.clone());
        }
        if !rows.is_empty() {
            self.chain
                .append_all(&rows)
                .map_err(|error| IntakeError::Store(error.to_string()))?;
        }
        Ok(rows)
    }

    /// Takes a file saved to `agent`'s inbox at `saved` (its
    /// workspace-relative path, unique per content) as an item of the
    /// Agent's push source: named by `filename`, with its text when it is
    /// text, labelled by detection like any other item. Taking the same
    /// saved file again returns its first row.
    pub fn take_push(
        &self,
        agent: &str,
        saved: &str,
        filename: &str,
        bytes: &[u8],
        trace: Option<&TraceKey>,
        actor: Option<PrincipalId>,
    ) -> Result<IntakeEvent, IntakeError> {
        let source = push_source(agent);
        let text: String = std::str::from_utf8(bytes)
            .ok()
            .filter(|text| !text.contains('\0'))
            .map(|text| text.chars().take(vak_intake::MAX_TEXT_CHARS).collect())
            .unwrap_or_default();
        let item = Item {
            key: saved.to_string(),
            title: filename.chars().take(500).collect(),
            link: None,
            author: None,
            published: None,
            text,
        };
        let id = item_id(&source, &item);
        if let Some(row) = self.taken(&id) {
            return Ok(row);
        }
        let body =
            serde_json::to_vec(&item).map_err(|error| IntakeError::Store(error.to_string()))?;
        let object = TenantObjects::for_tenant(&self.tenant_home)
            .and_then(|objects| objects.put(&body, &object_scope(&source)))
            .map_err(|error| IntakeError::Store(error.to_string()))?;
        let detection = vak_intake::detect(&item);
        let scope = object_scope(&source);
        let row = IntakeEvent {
            item: id,
            at: Utc::now(),
            trace: trace.cloned(),
            actor: actor.or_else(|| trace.and_then(|trace| trace.actor)),
            step: IntakeStep::Taken {
                source,
                agent: agent.to_string(),
                key: item.key.clone(),
                object,
                title: item.title.clone(),
                link: None,
                published: None,
                disposition: detection.disposition,
                labels: detection.labels,
                evidence: detection.evidence,
            },
        };
        // What the item says belongs to its source, like its body (plan
        // M7a-b); the chain keeps its ids, time and disposition.
        let mut stored =
            serde_json::to_value(&row).map_err(|error| IntakeError::Store(error.to_string()))?;
        vak_session::content::seal_fields_under(
            &mut stored,
            &scope,
            &["title", "link", "key", "evidence"],
        )
        .map_err(|error| IntakeError::Store(error.to_string()))?;
        self.chain
            .append(&stored)
            .map_err(|error| IntakeError::Store(error.to_string()))?;
        Ok(row)
    }

    /// Records that `actor` released or held `item`.
    pub fn decide(
        &self,
        item: &str,
        actor: PrincipalId,
        step: IntakeStep,
    ) -> Result<(), IntakeError> {
        if matches!(step, IntakeStep::Taken { .. }) {
            return Err(IntakeError::Invalid("only a poll takes an item".into()));
        }
        self.chain
            .append(&IntakeEvent {
                item: item.to_string(),
                at: Utc::now(),
                trace: None,
                actor: Some(actor),
                step,
            })
            .map_err(|error| IntakeError::Store(error.to_string()))
    }

    /// The body of an item a `taken` row names.
    pub fn body(&self, source: &SourceId, object: &ObjectRef) -> Result<Item, IntakeError> {
        let objects = TenantObjects::for_tenant(&self.tenant_home)
            .map_err(|error| IntakeError::Store(error.to_string()))?;
        let bytes = objects
            .get(object, &object_scope(source))
            .map_err(|error| IntakeError::Store(error.to_string()))?;
        serde_json::from_slice(&bytes).map_err(|error| IntakeError::Store(error.to_string()))
    }

    /// The `taken` row of `item`, when one exists.
    pub fn taken(&self, item: &str) -> Option<IntakeEvent> {
        let mut found = None;
        self.chain.scan(|mut row: serde_json::Value| {
            if row.get("item").and_then(serde_json::Value::as_str) != Some(item) {
                return true;
            }
            if let Ok(vak_session::content::Restored::Whole) =
                vak_session::content::restore(&mut row)
                && let Ok(row) = serde_json::from_value::<IntakeEvent>(row)
                && matches!(row.step, IntakeStep::Taken { .. })
            {
                found = Some(row);
                return false;
            }
            true
        });
        found
    }
}

/// What a source's cursor position holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Position {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_modified: Option<String>,
    /// The newest keys taken, newest last.
    #[serde(default)]
    seen: Vec<String>,
}

/// How many keys a cursor remembers: more than any one fetch returns.
const SEEN_KEYS: usize = 1024;

/// What one poll did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Polled {
    /// The source answered that nothing changed.
    pub unchanged: bool,
    pub fetched: usize,
    pub taken: usize,
    /// Of those taken, how many detection held back.
    pub held: usize,
    /// The items taken that reach the Agent, with their ids: what alerts
    /// match.
    #[serde(skip)]
    pub reached: Vec<(String, Item)>,
}

/// Polls `source` under the run `trace`: holds its cursor, fetches with a
/// conditional request, has the worker at `worker_exe` parse the bytes,
/// takes what is new, then moves the cursor.
pub async fn poll(
    intake: &Intake,
    cursors: &vak_session::cursors::Cursors,
    source: &Source,
    trace: &TraceKey,
    worker_exe: &Path,
) -> Result<Polled, IntakeError> {
    let owner = source.cursor_owner();
    let stream = source.id.to_string();
    match cursors
        .hold(&owner, Utc::now())
        .map_err(|error| IntakeError::Store(error.to_string()))?
    {
        vak_session::cursors::Hold::Held => {}
        _ => return Err(IntakeError::Busy),
    }
    let mut position: Position = cursors
        .get(&owner, &stream)
        .map_err(|error| IntakeError::Store(error.to_string()))?
        .and_then(|cursor| serde_json::from_str(&cursor.position).ok())
        .unwrap_or_default();
    let mut headers = Vec::new();
    if let Some(etag) = &position.etag {
        headers.push(("if-none-match", etag.clone()));
    }
    if let Some(modified) = &position.last_modified {
        headers.push(("if-modified-since", modified.clone()));
    }
    let fetched = vak_tools::webfetch::guarded_get(
        &source.connector.url(),
        &headers,
        vak_intake::MAX_FETCH_BYTES,
    )
    .await
    .map_err(IntakeError::Fetch)?;
    if fetched.status == 304 {
        return Ok(Polled {
            unchanged: true,
            ..Polled::default()
        });
    }
    if !(200..300).contains(&fetched.status) {
        return Err(IntakeError::Fetch(format!(
            "the source answered {}",
            fetched.status
        )));
    }
    let items = vak_tools::broker::parse_intake(worker_exe, &source.connector, &fetched.body)
        .await
        .map_err(IntakeError::Parse)?;
    let rows = intake.take(source, trace, &items, &mut position.seen)?;
    let excess = position.seen.len().saturating_sub(SEEN_KEYS);
    position.seen.drain(..excess);
    position.etag = fetched.etag;
    position.last_modified = fetched.last_modified;
    let encoded =
        serde_json::to_string(&position).map_err(|error| IntakeError::Store(error.to_string()))?;
    let moved = cursors
        .advance(&owner, &stream, &encoded, None)
        .map_err(|error| IntakeError::Store(error.to_string()))?;
    if !moved {
        return Err(IntakeError::Busy);
    }
    let reached = items
        .iter()
        .filter_map(|item| {
            let id = item_id(&source.id, item);
            rows.iter()
                .any(|row| {
                    row.item == id
                        && matches!(&row.step, IntakeStep::Taken { disposition, .. } if disposition.reaches_agent())
                })
                .then(|| (id, item.clone()))
        })
        .collect();
    Ok(Polled {
        unchanged: false,
        fetched: items.len(),
        taken: rows.len(),
        reached,
        held: rows
            .iter()
            .filter(|row| {
                matches!(&row.step, IntakeStep::Taken { disposition, .. } if !disposition.reaches_agent())
            })
            .count(),
    })
}
