//! Per-conversation work-topic metadata, stored as a sealed Agent Document.
//!
//! Topics organize turns; they do not contain copied conversation content or
//! replace the append-only ledger. Each accepted mutation creates one Document
//! version. The version records its actor and trace, while Document history
//! retains earlier mutation snapshots.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ids::PrincipalId, trace::TraceKey};

const SCHEMA: u32 = 1;

/// Exact auxiliary input, readable only while every contributing person's
/// scope remains readable. Each layer uses the existing sealed-content format.
#[derive(Debug, Serialize, Deserialize)]
pub struct StoredOrganisationRequest {
    request: serde_json::Value,
    contributor_scopes: Vec<String>,
}

impl StoredOrganisationRequest {
    pub fn seal(
        request: &vak_llm::ChatRequest,
        contributor_scopes: Vec<String>,
    ) -> Result<Self, crate::types::SessionError> {
        let mut value = serde_json::to_value(request)
            .map_err(|e| crate::types::SessionError::Objects(e.to_string()))?;
        for scope in &contributor_scopes {
            let mut wrapper = serde_json::json!({"request":value});
            crate::content::seal_fields_under(&mut wrapper, scope, &["request"])?;
            value = wrapper;
        }
        Ok(Self {
            request: value,
            contributor_scopes,
        })
    }

    pub fn restore(self) -> Result<vak_llm::ChatRequest, crate::types::SessionError> {
        let mut value = self.request;
        for _ in self.contributor_scopes.iter().rev() {
            if crate::content::restore(&mut value)? == crate::content::Restored::Erased {
                return Err(crate::types::SessionError::Erased(
                    "request contributor".into(),
                ));
            }
            value = value
                .get_mut("request")
                .map(serde_json::Value::take)
                .ok_or_else(|| {
                    crate::types::SessionError::Objects("invalid auxiliary request".into())
                })?;
        }
        serde_json::from_value(value)
            .map_err(|e| crate::types::SessionError::Objects(e.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TopicId(String);

impl TopicId {
    pub fn new() -> Self {
        Self(format!("top_{}", Uuid::now_v7().hyphenated()))
    }

    pub fn parse(value: &str) -> Result<Self, TopicError> {
        let body = value
            .strip_prefix("top_")
            .ok_or_else(|| TopicError::Invalid("topic id has the wrong prefix".into()))?;
        let uuid = Uuid::parse_str(body)
            .map_err(|_| TopicError::Invalid("topic id is not a UUIDv7".into()))?;
        if uuid.get_version_num() != 7 || body != uuid.hyphenated().to_string() {
            return Err(TopicError::Invalid(
                "topic id is not a canonical UUIDv7".into(),
            ));
        }
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self) -> Result<(), TopicError> {
        Self::parse(&self.0).map(|_| ())
    }
}

impl Default for TopicId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Topic {
    pub id: TopicId,
    pub title: String,
    pub archived: bool,
    pub pinned: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: PrincipalId,
    pub updated_by: PrincipalId,
    pub trace: TraceKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementSource {
    Model,
    Human,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    /// Raw canonical UUIDv7, matching the turn addresses in the conversation
    /// ledger and history APIs.
    pub turn_id: String,
    /// An authored name for this outcome.
    pub outcome: String,
    /// A user may assign an entire turn without naming one of its planned
    /// outcomes. Model placements on that turn cannot override this choice.
    #[serde(default)]
    pub whole_turn: bool,
    /// Empty means the outcome is intentionally unassigned.
    pub topic_ids: Vec<TopicId>,
    pub source: PlacementSource,
    pub actor: PrincipalId,
    pub trace: TraceKey,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationKind {
    Created,
    Renamed,
    Pinned,
    Unpinned,
    Archived,
    Reopened,
    AutomaticEnabled,
    AutomaticDisabled,
    ModelPlacement,
    OrganisationRequested,
    OrganisationAttempted,
    Organised,
    TurnRenamed,
    HumanPlacement,
}

/// Provenance for the mutation that produced this Document version. Earlier
/// snapshots remain available through the immutable Document history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MutationRecord {
    pub kind: MutationKind,
    pub target: Option<String>,
    pub actor: PrincipalId,
    pub trace: TraceKey,
    pub at: DateTime<Utc>,
}

/// Authored navigation metadata and the audit of its independent model step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnName {
    pub turn_id: String,
    pub title: Option<String>,
    pub source: PlacementSource,
    pub trigger: crate::ids::TriggerId,
    pub request: crate::objects::ObjectRef,
    #[serde(default)]
    pub known_topics: Vec<TopicId>,
    #[serde(default)]
    pub response: Option<crate::objects::ObjectRef>,
    pub trace: TraceKey,
}

/// Structured organisation output; topic ids reuse active Topics, while a
/// missing id asks to create the model-authored title in this transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Organisation {
    pub title: String,
    pub outcomes: Vec<OrganisedOutcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrganisedOutcome {
    pub outcome: String,
    pub topics: Vec<OrganisedTopic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrganisedTopic {
    pub id: Option<TopicId>,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopicSnapshot {
    pub schema: u32,
    pub topics: Vec<Topic>,
    pub placements: Vec<Placement>,
    #[serde(default)]
    pub turn_names: Vec<TurnName>,
    pub automatic: bool,
    pub revision: u64,
    pub last_mutation: Option<MutationRecord>,
}

impl Default for TopicSnapshot {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            topics: Vec::new(),
            placements: Vec::new(),
            turn_names: Vec::new(),
            automatic: true,
            revision: 0,
            last_mutation: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TopicError {
    #[error("invalid work-topic data: {0}")]
    Invalid(String),
    #[error("work-topic state changed; reload it before saving")]
    Conflict,
    #[error("automatic topic organization is disabled")]
    AutomaticDisabled,
    #[error("work-topic document failed: {0}")]
    Store(String),
}

/// A per-conversation organizational view. The path is owned by `scope`,
/// its Document versions are granted to that Agent, and their contents are
/// encrypted under this conversation's key.
#[derive(Debug, Clone)]
pub struct TopicStore {
    path: PathBuf,
    session_id: String,
    expected_revision: Option<u64>,
    require_automatic: bool,
}

impl TopicStore {
    pub fn new(
        scope: &vak_config::scope::AgentScope,
        session_id: &str,
    ) -> Result<Self, TopicError> {
        parse_uuid7(session_id, "conversation id")?;
        Ok(Self {
            path: scope.work_threads(session_id),
            session_id: session_id.to_string(),
            expected_revision: None,
            require_automatic: false,
        })
    }

    /// Return a handle that refuses a mutation if the snapshot has moved
    /// since the caller read revision `revision`. The comparison occurs
    /// inside the Document CAS closure, including after a conflicting retry.
    pub fn at_revision(mut self, revision: u64) -> Self {
        self.expected_revision = Some(revision);
        self
    }

    /// Return a handle that accepts mutations only while automatic topic
    /// organization is enabled. The check is made inside the document CAS
    /// closure, so disabling automation races safely with model actions.
    pub fn while_automatic(mut self) -> Self {
        self.require_automatic = true;
        self
    }

    pub fn snapshot(&self) -> Result<TopicSnapshot, TopicError> {
        let raw = crate::documents::read_sealed(
            &self.path,
            &crate::objects::conversation_scope(&self.session_id),
        )
        .map_err(TopicError::Store)?;
        let Some(raw) = raw else {
            return Ok(TopicSnapshot::default());
        };
        let snapshot: TopicSnapshot = serde_json::from_str(&raw)
            .map_err(|error| TopicError::Invalid(format!("cannot decode snapshot: {error}")))?;
        validate_snapshot(&snapshot, &self.session_id)?;
        Ok(snapshot)
    }

    /// Register an independent organisation request once. A pending request
    /// survives restart through its Trigger; an explicit retry supplies a new
    /// trigger and input. A human title is always retained.
    pub fn request_organisation(
        &self,
        mut name: TurnName,
        retry: bool,
    ) -> Result<bool, TopicError> {
        parse_uuid7(&name.turn_id, "turn id")?;
        let trace = name.trace.clone();
        self.mutate(
            &trace,
            MutationKind::OrganisationRequested,
            Some(name.turn_id.clone()),
            |snapshot, _| {
                if let Some(existing) = snapshot
                    .turn_names
                    .iter_mut()
                    .find(|row| row.turn_id == name.turn_id)
                {
                    if !retry {
                        return Ok(None);
                    }
                    if existing.source == PlacementSource::Human {
                        name.title = existing.title.clone();
                        name.source = PlacementSource::Human;
                    }
                    *existing = name.clone();
                } else {
                    snapshot.turn_names.push(name.clone());
                }
                Ok(Some(()))
            },
        )
        .map(|changed| changed.is_some())
    }

    pub fn rename_turn(&self, turn: &str, title: &str, trace: &TraceKey) -> Result<(), TopicError> {
        let title = clean_title(title)?;
        self.mutate(
            trace,
            MutationKind::TurnRenamed,
            Some(turn.into()),
            |snapshot, _| {
                let row = snapshot
                    .turn_names
                    .iter_mut()
                    .find(|row| row.turn_id == turn)
                    .ok_or_else(|| TopicError::Invalid("turn name is unavailable".into()))?;
                row.title = Some(title.clone());
                row.source = PlacementSource::Human;
                row.trace = trace.clone();
                Ok(Some(()))
            },
        )
        .map(|_| ())
    }

    /// Preserve the exact failed model response/receipt under the source key.
    /// Status remains a query over Runs; this is audit evidence, not a queue.
    pub fn note_organisation_attempt(
        &self,
        turn: &str,
        trigger: crate::ids::TriggerId,
        response: crate::objects::ObjectRef,
        trace: &TraceKey,
    ) -> Result<(), TopicError> {
        self.mutate(
            trace,
            MutationKind::OrganisationAttempted,
            Some(turn.into()),
            |snapshot, _| {
                let Some(row) = snapshot.turn_names.iter_mut().find(|row| {
                    row.turn_id == turn && row.trigger == trigger && row.response.is_none()
                }) else {
                    return Ok(None);
                };
                row.response = Some(response.clone());
                Ok(Some(()))
            },
        )
        .map(|_| ())
    }

    /// Apply the title, new Topics and all placements in one sealed Document
    /// version. Recheck human corrections, automatic-off, archived Topics and
    /// the exact pending trigger inside CAS; a late run cannot replace a retry.
    pub fn organise(
        &self,
        turn: &str,
        trigger: crate::ids::TriggerId,
        result: &Organisation,
        response: crate::objects::ObjectRef,
        trace: &TraceKey,
    ) -> Result<bool, TopicError> {
        let title = clean_title(&result.title)?;
        for outcome in &result.outcomes {
            clean_title(&outcome.outcome)?;
            for topic in &outcome.topics {
                clean_title(&topic.title)?;
            }
        }
        // Stable across CAS retries, including references shared by outcomes.
        let mut new_ids = std::collections::BTreeMap::new();
        for topic in result
            .outcomes
            .iter()
            .flat_map(|o| &o.topics)
            .filter(|t| t.id.is_none())
        {
            new_ids
                .entry(topic.title.trim().to_string())
                .or_insert_with(TopicId::new);
        }
        self.mutate(
            trace,
            MutationKind::Organised,
            Some(turn.into()),
            |snapshot, actor| {
                let Some(index) = snapshot.turn_names.iter().position(|row| {
                    row.turn_id == turn && row.trigger == trigger && row.response.is_none()
                }) else {
                    return Ok(None);
                };
                if snapshot.automatic && result.outcomes.is_empty() {
                    return Err(TopicError::Invalid(
                        "automatic organisation requires an explicit outcome decision".into(),
                    ));
                }
                let mut outcomes = std::collections::HashSet::new();
                if result
                    .outcomes
                    .iter()
                    .any(|outcome| !outcomes.insert(outcome.outcome.trim()))
                {
                    return Err(TopicError::Invalid("duplicate outcome decision".into()));
                }
                let human_turn = snapshot.placements.iter().any(|p| {
                    p.turn_id == turn && p.source == PlacementSource::Human && p.whole_turn
                });
                if snapshot.automatic && !human_turn {
                    // This result is a complete model decision for the turn.
                    // Retries replace prior model outcomes while preserving
                    // every human correction, including explicit ungrouping.
                    snapshot
                        .placements
                        .retain(|p| p.turn_id != turn || p.source != PlacementSource::Model);
                    for outcome in &result.outcomes {
                        if snapshot.placements.iter().any(|p| {
                            p.turn_id == turn
                                && p.source == PlacementSource::Human
                                && p.outcome == outcome.outcome.trim()
                        }) {
                            continue;
                        }
                        let mut ids = Vec::new();
                        for choice in &outcome.topics {
                            let id = if let Some(id) = &choice.id {
                                if !snapshot.turn_names[index].known_topics.contains(id) {
                                    return Err(TopicError::Invalid(
                                        "topic was not in the recorded organisation input".into(),
                                    ));
                                }
                                if find_topic(snapshot, id)?.archived {
                                    return Err(TopicError::Invalid(
                                        "archived topic cannot receive a placement".into(),
                                    ));
                                }
                                id.clone()
                            } else {
                                // Concurrent proposals of the same new name share
                                // one identity. Names already in the input are
                                // selected by id, never matched by their words.
                                let concurrent = snapshot.topics.iter().find(|topic| {
                                    !topic.archived
                                        && !snapshot.turn_names[index]
                                            .known_topics
                                            .contains(&topic.id)
                                        && topic.title == choice.title.trim()
                                });
                                let id = concurrent
                                    .map(|topic| topic.id.clone())
                                    .or_else(|| new_ids.get(choice.title.trim()).cloned())
                                    .ok_or_else(|| {
                                        TopicError::Invalid("new topic has no id".into())
                                    })?;
                                if !snapshot.topics.iter().any(|topic| topic.id == id) {
                                    snapshot.topics.push(Topic {
                                        id: id.clone(),
                                        title: choice.title.trim().into(),
                                        archived: false,
                                        pinned: false,
                                        created_at: Utc::now(),
                                        updated_at: Utc::now(),
                                        created_by: *actor,
                                        updated_by: *actor,
                                        trace: trace.clone(),
                                    });
                                }
                                id
                            };
                            if !ids.contains(&id) {
                                ids.push(id);
                            }
                        }
                        snapshot.placements.push(Placement {
                            turn_id: turn.into(),
                            outcome: outcome.outcome.trim().into(),
                            whole_turn: false,
                            topic_ids: ids,
                            source: PlacementSource::Model,
                            actor: *actor,
                            trace: trace.clone(),
                            updated_at: Utc::now(),
                        });
                    }
                }
                let row = &mut snapshot.turn_names[index];
                if row.source != PlacementSource::Human {
                    row.title = Some(title.clone());
                    row.trace = trace.clone();
                }
                row.response = Some(response.clone());
                Ok(Some(()))
            },
        )
        .map(|changed| changed.is_some())
    }

    pub fn get(&self, id: &TopicId) -> Result<Topic, TopicError> {
        id.validate()?;
        self.snapshot()?
            .topics
            .into_iter()
            .find(|topic| &topic.id == id)
            .ok_or_else(|| TopicError::Invalid("topic does not exist".into()))
    }

    pub fn create(
        &self,
        title: &str,
        pinned: bool,
        trace: &TraceKey,
    ) -> Result<TopicId, TopicError> {
        let title = clean_title(title)?;
        let id = TopicId::new(); // minted once, outside the CAS retry closure
        let created = self.mutate(
            trace,
            MutationKind::Created,
            Some(id.as_str().into()),
            |snapshot, actor| {
                let now = Utc::now();
                snapshot.topics.push(Topic {
                    id: id.clone(),
                    title: title.clone(),
                    archived: false,
                    pinned,
                    created_at: now,
                    updated_at: now,
                    created_by: *actor,
                    updated_by: *actor,
                    trace: trace.clone(),
                });
                Ok(Some(id.clone()))
            },
        )?;
        created.ok_or_else(|| TopicError::Invalid("topic creation was not recorded".into()))
    }

    /// Atomically creates a topic and places the current model outcomes in it.
    /// The topic id is minted once outside the CAS retry closure, so a retry
    /// cannot create a second topic. A human correction on this turn wins over
    /// this operation as a whole; no orphan topic is written.
    pub fn create_for_outcomes(
        &self,
        title: &str,
        pinned: bool,
        turn_id: &str,
        outcomes: &[String],
        trace: &TraceKey,
    ) -> Result<TopicId, TopicError> {
        let title = clean_title(title)?;
        parse_uuid7(turn_id, "turn id")?;
        if trace
            .turn
            .is_none_or(|turn| turn.uuid().hyphenated().to_string() != turn_id)
        {
            return Err(TopicError::Invalid(
                "mutation trace does not name the requested turn".into(),
            ));
        }
        let mut unique = std::collections::HashSet::new();
        let outcomes = outcomes
            .iter()
            .map(|outcome| clean_title(outcome))
            .collect::<Result<Vec<_>, _>>()?;
        if outcomes.is_empty() || outcomes.iter().any(|outcome| !unique.insert(outcome)) {
            return Err(TopicError::Invalid(
                "outcomes must be nonempty and unique".into(),
            ));
        }

        let turn_id = turn_id.to_string();
        let id = TopicId::new(); // stable across CAS retries
        let created = self.mutate(
            trace,
            MutationKind::Created,
            Some(id.as_str().into()),
            |snapshot, actor| {
                if !snapshot.automatic {
                    return Err(TopicError::AutomaticDisabled);
                }
                if snapshot.placements.iter().any(|placement| {
                    placement.turn_id == turn_id
                        && placement.source == PlacementSource::Human
                        && (placement.whole_turn || outcomes.contains(&placement.outcome))
                }) {
                    return Err(TopicError::Invalid("__topic_human_corrected__".into()));
                }

                let now = Utc::now();
                snapshot.topics.push(Topic {
                    id: id.clone(),
                    title: title.clone(),
                    archived: false,
                    pinned,
                    created_at: now,
                    updated_at: now,
                    created_by: *actor,
                    updated_by: *actor,
                    trace: trace.clone(),
                });
                for outcome in &outcomes {
                    if let Some(current) = snapshot.placements.iter_mut().find(|placement| {
                        placement.turn_id == turn_id
                            && !placement.whole_turn
                            && placement.outcome == *outcome
                    }) {
                        current.topic_ids = vec![id.clone()];
                        current.source = PlacementSource::Model;
                        current.actor = *actor;
                        current.trace = trace.clone();
                        current.updated_at = now;
                    } else {
                        snapshot.placements.push(Placement {
                            turn_id: turn_id.clone(),
                            outcome: outcome.clone(),
                            whole_turn: false,
                            topic_ids: vec![id.clone()],
                            source: PlacementSource::Model,
                            actor: *actor,
                            trace: trace.clone(),
                            updated_at: now,
                        });
                    }
                }
                Ok(Some(id.clone()))
            },
        )?;
        created.ok_or_else(|| TopicError::Invalid("topic creation was not recorded".into()))
    }

    pub fn rename(&self, id: &TopicId, title: &str, trace: &TraceKey) -> Result<(), TopicError> {
        id.validate()?;
        let title = clean_title(title)?;
        self.mutate(
            trace,
            MutationKind::Renamed,
            Some(id.as_str().into()),
            |snapshot, actor| {
                let topic = find_topic_mut(snapshot, id)?;
                topic.title = title.clone();
                topic.updated_at = Utc::now();
                topic.updated_by = *actor;
                topic.trace = trace.clone();
                Ok(Some(()))
            },
        )
        .map(|_| ())
    }

    pub fn set_pinned(
        &self,
        id: &TopicId,
        pinned: bool,
        trace: &TraceKey,
    ) -> Result<(), TopicError> {
        id.validate()?;
        let kind = if pinned {
            MutationKind::Pinned
        } else {
            MutationKind::Unpinned
        };
        self.mutate(trace, kind, Some(id.as_str().into()), |snapshot, actor| {
            let topic = find_topic_mut(snapshot, id)?;
            topic.pinned = pinned;
            topic.updated_at = Utc::now();
            topic.updated_by = *actor;
            topic.trace = trace.clone();
            Ok(Some(()))
        })
        .map(|_| ())
    }

    pub fn archive(&self, id: &TopicId, trace: &TraceKey) -> Result<(), TopicError> {
        self.set_archived(id, true, trace)
    }

    pub fn reopen(&self, id: &TopicId, trace: &TraceKey) -> Result<(), TopicError> {
        self.set_archived(id, false, trace)
    }

    pub fn set_automatic(&self, automatic: bool, trace: &TraceKey) -> Result<(), TopicError> {
        let kind = if automatic {
            MutationKind::AutomaticEnabled
        } else {
            MutationKind::AutomaticDisabled
        };
        self.mutate(trace, kind, None, |snapshot, _| {
            if snapshot.automatic == automatic {
                return Ok(None);
            }
            snapshot.automatic = automatic;
            Ok(Some(()))
        })
        .map(|_| ())
    }

    /// Stores the model's placement unless a human has corrected this exact
    /// turn/outcome. Archived topics cannot receive new placements.
    pub fn assign_model(
        &self,
        turn_id: &str,
        outcome: &str,
        topic_ids: Vec<TopicId>,
        trace: &TraceKey,
    ) -> Result<bool, TopicError> {
        self.assign(
            turn_id,
            outcome,
            topic_ids,
            false,
            PlacementSource::Model,
            trace,
        )
    }

    /// A human correction replaces either a model placement or an earlier
    /// human placement for the exact turn/outcome.
    pub fn assign_human(
        &self,
        turn_id: &str,
        outcome: &str,
        topic_ids: Vec<TopicId>,
        trace: &TraceKey,
    ) -> Result<bool, TopicError> {
        self.assign(
            turn_id,
            outcome,
            topic_ids,
            false,
            PlacementSource::Human,
            trace,
        )
    }

    /// Assigns the whole turn to topics. This human choice takes precedence
    /// over every model outcome placement for the turn.
    pub fn assign_turn_human(
        &self,
        turn_id: &str,
        topic_ids: Vec<TopicId>,
        trace: &TraceKey,
    ) -> Result<bool, TopicError> {
        self.assign(
            turn_id,
            "This request",
            topic_ids,
            true,
            PlacementSource::Human,
            trace,
        )
    }

    fn assign(
        &self,
        turn_id: &str,
        outcome: &str,
        topic_ids: Vec<TopicId>,
        whole_turn: bool,
        source: PlacementSource,
        trace: &TraceKey,
    ) -> Result<bool, TopicError> {
        parse_uuid7(turn_id, "turn id")?;
        let turn_id = turn_id.to_string();
        let outcome = clean_title(outcome)?;
        validate_topic_ids(&topic_ids)?;
        let kind = match source {
            PlacementSource::Model => MutationKind::ModelPlacement,
            PlacementSource::Human => MutationKind::HumanPlacement,
        };
        self.mutate(trace, kind, Some(turn_id.to_string()), |snapshot, actor| {
            if source == PlacementSource::Model && !snapshot.automatic {
                return Ok(None);
            }
            if source == PlacementSource::Model
                && snapshot.placements.iter().any(|placement| {
                    placement.turn_id == turn_id
                        && placement.whole_turn
                        && placement.source == PlacementSource::Human
                })
            {
                return Ok(None);
            }
            let previous_membership = snapshot
                .placements
                .iter()
                .filter(|placement| {
                    placement.turn_id == turn_id
                        && (whole_turn
                            || (placement.whole_turn == whole_turn && placement.outcome == outcome))
                })
                .flat_map(|placement| placement.topic_ids.iter().cloned())
                .collect::<std::collections::HashSet<_>>();
            for id in &topic_ids {
                let topic = find_topic(snapshot, id)?;
                if topic.archived
                    && !(source == PlacementSource::Human && previous_membership.contains(id))
                {
                    return Err(TopicError::Invalid(
                        "archived topic cannot receive a placement".into(),
                    ));
                }
            }
            // A person's whole-request correction supersedes every earlier
            // outcome placement. Keeping those rows in the current view
            // would leave a moved or unassigned request in its old Topics.
            // Immutable Document history retains the earlier organisation.
            if whole_turn && source == PlacementSource::Human {
                snapshot
                    .placements
                    .retain(|placement| placement.turn_id != turn_id);
            }
            if let Some(current) = snapshot.placements.iter_mut().find(|placement| {
                placement.turn_id == turn_id
                    && placement.whole_turn == whole_turn
                    && placement.outcome == outcome
            }) {
                if source == PlacementSource::Model && current.source == PlacementSource::Human {
                    return Ok(None);
                }
                *current = Placement {
                    turn_id: turn_id.clone(),
                    outcome: outcome.clone(),
                    whole_turn,
                    topic_ids: topic_ids.clone(),
                    source,
                    actor: *actor,
                    trace: trace.clone(),
                    updated_at: Utc::now(),
                };
            } else {
                snapshot.placements.push(Placement {
                    turn_id: turn_id.clone(),
                    outcome: outcome.clone(),
                    whole_turn,
                    topic_ids: topic_ids.clone(),
                    source,
                    actor: *actor,
                    trace: trace.clone(),
                    updated_at: Utc::now(),
                });
            }
            Ok(Some(true))
        })
        .map(|stored| stored.unwrap_or(false))
    }

    fn set_archived(
        &self,
        id: &TopicId,
        archived: bool,
        trace: &TraceKey,
    ) -> Result<(), TopicError> {
        id.validate()?;
        let kind = if archived {
            MutationKind::Archived
        } else {
            MutationKind::Reopened
        };
        self.mutate(trace, kind, Some(id.as_str().into()), |snapshot, actor| {
            let topic = find_topic_mut(snapshot, id)?;
            topic.archived = archived;
            topic.updated_at = Utc::now();
            topic.updated_by = *actor;
            topic.trace = trace.clone();
            Ok(Some(()))
        })
        .map(|_| ())
    }

    fn mutate<T>(
        &self,
        trace: &TraceKey,
        kind: MutationKind,
        target: Option<String>,
        mut change: impl FnMut(&mut TopicSnapshot, &PrincipalId) -> Result<Option<T>, TopicError>,
    ) -> Result<Option<T>, TopicError> {
        let actor = trace
            .actor
            .ok_or_else(|| TopicError::Invalid("mutation trace has no actor".into()))?;
        if trace.session_id().as_deref() != Some(&self.session_id) {
            return Err(TopicError::Invalid(
                "mutation trace belongs to another conversation".into(),
            ));
        }
        let raw = crate::documents::update_sealed(
            &self.path,
            &crate::objects::conversation_scope(&self.session_id),
            |current| {
                let mut snapshot = match current {
                    Some(raw) => {
                        let snapshot: TopicSnapshot = serde_json::from_str(raw)
                            .map_err(|error| format!("invalid work-topic snapshot: {error}"))?;
                        validate_snapshot(&snapshot, &self.session_id)
                            .map_err(|error| error.to_string())?;
                        snapshot
                    }
                    None => TopicSnapshot::default(),
                };
                if self
                    .expected_revision
                    .is_some_and(|expected| expected != snapshot.revision)
                {
                    return Err("__topic_conflict__".into());
                }
                if self.require_automatic && !snapshot.automatic {
                    return Err("__topic_automatic_disabled__".into());
                }
                let Some(result) = change(&mut snapshot, &actor).map_err(|error| match error {
                    TopicError::AutomaticDisabled => "__topic_automatic_disabled__".into(),
                    TopicError::Invalid(message) if message == "__topic_human_corrected__" => {
                        "__topic_human_corrected__".into()
                    }
                    error => error.to_string(),
                })?
                else {
                    return Ok(None);
                };
                snapshot.revision = snapshot
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| "work-topic revision overflow".to_string())?;
                snapshot.last_mutation = Some(MutationRecord {
                    kind: kind.clone(),
                    target: target.clone(),
                    actor,
                    trace: trace.clone(),
                    at: Utc::now(),
                });
                let encoded = serde_json::to_string(&snapshot)
                    .map_err(|error| format!("cannot encode work-topic snapshot: {error}"))?;
                Ok(Some((encoded, result)))
            },
        )
        .map_err(|error| {
            if error == "__topic_conflict__" {
                TopicError::Conflict
            } else if error == "__topic_automatic_disabled__" {
                TopicError::AutomaticDisabled
            } else if error == "__topic_human_corrected__" {
                TopicError::Invalid(
                    "a human correction already controls this turn or outcome".into(),
                )
            } else {
                TopicError::Store(error)
            }
        })?;
        Ok(raw)
    }
}

fn clean_title(title: &str) -> Result<String, TopicError> {
    let title = title.trim();
    if title.is_empty() {
        Err(TopicError::Invalid("title or outcome is empty".into()))
    } else {
        Ok(title.to_string())
    }
}

fn parse_uuid7(value: &str, label: &str) -> Result<Uuid, TopicError> {
    let uuid = Uuid::parse_str(value)
        .map_err(|_| TopicError::Invalid(format!("{label} is not a UUIDv7")))?;
    if uuid.get_version_num() != 7 || value != uuid.hyphenated().to_string() {
        return Err(TopicError::Invalid(format!(
            "{label} is not a canonical UUIDv7"
        )));
    }
    Ok(uuid)
}

fn validate_topic_ids(ids: &[TopicId]) -> Result<(), TopicError> {
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        id.validate()?;
        if !seen.insert(id) {
            return Err(TopicError::Invalid("topic id is repeated".into()));
        }
    }
    Ok(())
}

fn find_topic<'a>(snapshot: &'a TopicSnapshot, id: &TopicId) -> Result<&'a Topic, TopicError> {
    snapshot
        .topics
        .iter()
        .find(|topic| &topic.id == id)
        .ok_or_else(|| TopicError::Invalid("topic does not exist".into()))
}

fn find_topic_mut<'a>(
    snapshot: &'a mut TopicSnapshot,
    id: &TopicId,
) -> Result<&'a mut Topic, TopicError> {
    snapshot
        .topics
        .iter_mut()
        .find(|topic| &topic.id == id)
        .ok_or_else(|| TopicError::Invalid("topic does not exist".into()))
}

fn validate_snapshot(snapshot: &TopicSnapshot, session_id: &str) -> Result<(), TopicError> {
    if snapshot.schema != SCHEMA {
        return Err(TopicError::Invalid("unsupported snapshot schema".into()));
    }
    let mut names = std::collections::HashSet::new();
    for row in &snapshot.turn_names {
        parse_uuid7(&row.turn_id, "turn id")?;
        if !names.insert(&row.turn_id)
            || row
                .title
                .as_ref()
                .is_some_and(|title| title.trim().is_empty())
            || row.trace.session_id().as_deref() != Some(session_id)
        {
            return Err(TopicError::Invalid(
                "invalid turn name provenance or duplicate turn".into(),
            ));
        }
    }
    let mut topics = std::collections::HashSet::new();
    for topic in &snapshot.topics {
        topic.id.validate()?;
        if !topics.insert(&topic.id)
            || topic.title.trim().is_empty()
            || topic.trace.session_id().as_deref() != Some(session_id)
            || topic.trace.actor.as_ref() != Some(&topic.updated_by)
        {
            return Err(TopicError::Invalid("duplicate topic or empty title".into()));
        }
    }
    let mut placements = std::collections::HashSet::new();
    for placement in &snapshot.placements {
        parse_uuid7(&placement.turn_id, "turn id")?;
        if placement.trace.session_id().as_deref() != Some(session_id)
            || placement.trace.actor.as_ref() != Some(&placement.actor)
        {
            return Err(TopicError::Invalid(
                "placement provenance belongs to another actor or conversation".into(),
            ));
        }
        for id in &placement.topic_ids {
            id.validate()?;
            if !topics.contains(id) {
                return Err(TopicError::Invalid(
                    "placement names a missing topic".into(),
                ));
            }
        }
        if placement.outcome.trim().is_empty()
            || (placement.whole_turn && placement.source != PlacementSource::Human)
            || !placements.insert((
                placement.turn_id.as_str(),
                placement.outcome.as_str(),
                placement.whole_turn,
            ))
        {
            return Err(TopicError::Invalid(
                "duplicate placement or empty outcome".into(),
            ));
        }
    }
    if snapshot.revision == 0
        && (snapshot.last_mutation.is_some()
            || !snapshot.topics.is_empty()
            || !snapshot.placements.is_empty()
            || !snapshot.turn_names.is_empty())
    {
        return Err(TopicError::Invalid(
            "initial revision contains mutations".into(),
        ));
    }
    if snapshot.revision > 0 && snapshot.last_mutation.is_none() {
        return Err(TopicError::Invalid(
            "snapshot mutation provenance is missing".into(),
        ));
    }
    if snapshot.last_mutation.as_ref().is_some_and(|mutation| {
        mutation.trace.session_id().as_deref() != Some(session_id)
            || mutation.trace.actor.as_ref() != Some(&mutation.actor)
    }) {
        return Err(TopicError::Invalid(
            "mutation provenance belongs to another actor or conversation".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::trace::{self, Cause};
    use std::path::Path;

    fn setup() -> (TopicStore, String, TraceKey) {
        let home = vak_config::paths::isolate_home_for_tests();
        let scope = vak_config::scope::AgentScope::new(vak_config::paths::agent_home_at(
            &home,
            "topics-test-agent",
        ));
        let session_id = Uuid::now_v7().hyphenated().to_string();
        let turn_id = Uuid::now_v7().hyphenated().to_string();
        let trace = TraceKey::root(
            trace::local::tenant(),
            trace::local::space(Path::new("/tmp/topics-test-workspace")),
            trace::local::agent("topics-test-agent"),
            Cause::User {
                request_id: Uuid::now_v7().hyphenated().to_string(),
            },
        )
        .in_turn(&session_id, &turn_id)
        .acting(trace::local::local_owner(), None);
        (
            TopicStore::new(&scope, &session_id).unwrap(),
            turn_id,
            trace,
        )
    }

    fn pending(store: &TopicStore, turn: &str, trace: &TraceKey) -> crate::ids::TriggerId {
        let trigger = crate::ids::TriggerId::new();
        assert!(
            store
                .request_organisation(
                    TurnName {
                        turn_id: turn.into(),
                        title: None,
                        source: PlacementSource::Model,
                        trigger,
                        known_topics: store
                            .snapshot()
                            .unwrap()
                            .topics
                            .into_iter()
                            .map(|topic| topic.id)
                            .collect(),
                        request: crate::objects::ObjectRef {
                            id: "input".into(),
                            len: 5
                        },
                        response: None,
                        trace: trace.clone()
                    },
                    false
                )
                .unwrap()
        );
        trigger
    }

    fn response() -> crate::objects::ObjectRef {
        crate::objects::ObjectRef {
            id: "response".into(),
            len: 8,
        }
    }

    #[test]
    fn auxiliary_input_roundtrips_through_every_contributor_scope() {
        let (_store, _turn, trace) = setup();
        let session = trace.session_id().unwrap();
        let scopes = [
            crate::ids::PrincipalId::new(),
            crate::ids::PrincipalId::new(),
        ]
        .iter()
        .map(|author| crate::objects::contributor_scope(&session, &author.to_string()))
        .collect();
        let mut request = vak_llm::ChatRequest::new("fixture");
        request
            .messages
            .push(vak_llm::Message::user_text("Private supplied context"));
        let stored = StoredOrganisationRequest::seal(&request, scopes).unwrap();
        let encoded = serde_json::to_string(&stored).unwrap();
        assert!(!encoded.contains("Private supplied context"));
        let decoded: StoredOrganisationRequest = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            serde_json::to_value(decoded.restore().unwrap()).unwrap(),
            serde_json::to_value(request).unwrap()
        );
    }

    #[test]
    fn organisation_names_plain_answers_and_creates_shared_topics_atomically() {
        let (store, turn, trace) = setup();
        store
            .assign_model(&turn, "Earlier draft grouping", vec![], &trace)
            .unwrap();
        let trigger = pending(&store, &turn, &trace);
        let result = Organisation {
            title: "A useful work name".into(),
            outcomes: vec![
                OrganisedOutcome {
                    outcome: "Research".into(),
                    topics: vec![OrganisedTopic {
                        id: None,
                        title: "Ongoing work".into(),
                    }],
                },
                OrganisedOutcome {
                    outcome: "Write the brief".into(),
                    topics: vec![OrganisedTopic {
                        id: None,
                        title: "Ongoing work".into(),
                    }],
                },
            ],
        };
        let before = store.snapshot().unwrap().revision;
        assert!(
            store
                .organise(&turn, trigger, &result, response(), &trace)
                .unwrap()
        );
        let after = store.snapshot().unwrap();
        assert_eq!(after.revision, before + 1);
        assert_eq!(
            after.turn_names[0].title.as_deref(),
            Some("A useful work name")
        );
        assert_eq!(after.topics.len(), 1);
        assert_eq!(after.placements.len(), 2);
        assert!(
            after
                .placements
                .iter()
                .all(|p| p.topic_ids == vec![after.topics[0].id.clone()])
        );
        assert!(
            !store
                .organise(&turn, trigger, &result, response(), &trace)
                .unwrap()
        );
    }

    #[test]
    fn late_organisation_preserves_human_names_placement_and_automatic_off() {
        for automatic in [false, true] {
            let (store, turn, trace) = setup();
            let trigger = pending(&store, &turn, &trace);
            store.rename_turn(&turn, "My chosen name", &trace).unwrap();
            if automatic {
                store.assign_turn_human(&turn, vec![], &trace).unwrap();
            } else {
                store.set_automatic(false, &trace).unwrap();
            }
            let result = Organisation {
                title: "Model name".into(),
                outcomes: vec![OrganisedOutcome {
                    outcome: "A result".into(),
                    topics: vec![OrganisedTopic {
                        id: None,
                        title: "Must not exist".into(),
                    }],
                }],
            };
            assert!(
                store
                    .organise(&turn, trigger, &result, response(), &trace)
                    .unwrap()
            );
            let state = store.snapshot().unwrap();
            assert_eq!(state.turn_names[0].title.as_deref(), Some("My chosen name"));
            assert!(state.topics.is_empty());
            assert!(
                state
                    .placements
                    .iter()
                    .all(|p| p.source == PlacementSource::Human)
            );
        }
    }

    #[test]
    fn archived_or_missing_topic_rejects_the_whole_result_and_retry_fences_old_results() {
        let (store, turn, trace) = setup();
        let topic = store.create("Past work", false, &trace).unwrap();
        let trigger = pending(&store, &turn, &trace);
        store.archive(&topic, &trace).unwrap();
        let result = Organisation {
            title: "Must not partly save".into(),
            outcomes: vec![OrganisedOutcome {
                outcome: "A result".into(),
                topics: vec![OrganisedTopic {
                    id: Some(topic),
                    title: "Past work".into(),
                }],
            }],
        };
        let before = store.snapshot().unwrap();
        assert!(
            store
                .organise(&turn, trigger, &result, response(), &trace)
                .is_err()
        );
        assert_eq!(store.snapshot().unwrap(), before);
        let retry = crate::ids::TriggerId::new();
        let mut next = before.turn_names[0].clone();
        next.trigger = retry;
        assert!(store.request_organisation(next, true).unwrap());
        assert!(
            !store
                .organise(
                    &turn,
                    trigger,
                    &Organisation {
                        title: "Old result".into(),
                        outcomes: vec![]
                    },
                    response(),
                    &trace
                )
                .unwrap()
        );
        assert_eq!(store.snapshot().unwrap().turn_names[0].trigger, retry);
    }

    #[test]
    fn a_whole_request_move_or_unassignment_replaces_earlier_outcome_membership() {
        let (store, turn, trace) = setup();
        let first = store.create("Earlier work", false, &trace).unwrap();
        let second = store.create("Chosen work", false, &trace).unwrap();
        store
            .assign_model(&turn, "first outcome", vec![first.clone()], &trace)
            .unwrap();
        store
            .assign_model(&turn, "second outcome", vec![first], &trace)
            .unwrap();
        store
            .assign_turn_human(&turn, vec![second.clone()], &trace)
            .unwrap();
        let moved = store.snapshot().unwrap();
        assert_eq!(moved.placements.len(), 1);
        assert!(moved.placements[0].whole_turn);
        assert_eq!(moved.placements[0].topic_ids, vec![second.clone()]);
        store.assign_turn_human(&turn, Vec::new(), &trace).unwrap();
        assert!(
            store
                .snapshot()
                .unwrap()
                .placements
                .iter()
                .all(|p| p.topic_ids.is_empty())
        );
        assert!(
            !store
                .assign_model(&turn, "renamed outcome", vec![second], &trace)
                .unwrap()
        );
    }

    #[test]
    fn model_can_assign_many_topics_and_human_correction_wins_atomically() {
        let (store, turn, trace) = setup();
        let research = store.create("Research", false, &trace).unwrap();
        let writing = store.create("Writing", true, &trace).unwrap();
        assert!(
            store
                .assign_model(
                    &turn,
                    "source review",
                    vec![research.clone(), writing.clone()],
                    &trace
                )
                .unwrap()
        );
        assert!(
            store
                .assign_model(&turn, "final draft", vec![writing.clone()], &trace)
                .unwrap()
        );
        let observed = store.snapshot().unwrap();
        let first = observed
            .placements
            .iter()
            .find(|placement| placement.outcome == "source review")
            .unwrap();
        assert_eq!(first.topic_ids, vec![research.clone(), writing.clone()]);
        assert_eq!(first.source, PlacementSource::Model);
        assert!(observed.placements.iter().any(|placement| {
            placement.outcome == "final draft" && placement.topic_ids == [writing.clone()]
        }));

        assert!(
            store
                .clone()
                .at_revision(observed.revision)
                .assign_human(&turn, "source review", vec![writing.clone()], &trace)
                .unwrap()
        );
        assert!(matches!(
            store.clone().at_revision(observed.revision).assign_human(
                &turn,
                "source review",
                vec![research.clone()],
                &trace
            ),
            Err(TopicError::Conflict)
        ));
        assert!(
            !store
                .assign_model(&turn, "source review", vec![research], &trace)
                .unwrap()
        );

        let final_state = store.snapshot().unwrap();
        assert_eq!(final_state.revision, observed.revision + 1);
        assert_eq!(final_state.placements[0].topic_ids, vec![writing]);
        assert_eq!(final_state.placements[0].source, PlacementSource::Human);
        assert_eq!(
            final_state.last_mutation.unwrap().actor,
            trace::local::local_owner()
        );
    }

    #[test]
    fn archived_topics_need_explicit_reopen_and_auto_mode_can_be_disabled() {
        let (store, turn, trace) = setup();
        let topic = store.create("Ongoing work", false, &trace).unwrap();
        assert!(matches!(
            store.assign_model(&turn, "unknown", vec![TopicId::new()], &trace),
            Err(TopicError::Store(_))
        ));
        store.archive(&topic, &trace).unwrap();
        assert!(matches!(
            store.assign_model(&turn, "follow-up", vec![topic.clone()], &trace),
            Err(TopicError::Store(_))
        ));
        assert!(store.get(&topic).unwrap().archived);
        store.reopen(&topic, &trace).unwrap();
        store.set_automatic(false, &trace).unwrap();
        assert!(
            !store
                .assign_model(&turn, "follow-up", vec![topic.clone()], &trace)
                .unwrap()
        );
        assert!(
            store
                .assign_human(&turn, "follow-up", vec![topic], &trace)
                .unwrap()
        );
        assert_eq!(store.snapshot().unwrap().placements.len(), 1);
    }

    #[test]
    fn model_mutations_recheck_automatic_mode_inside_the_document_update() {
        let (store, _, trace) = setup();
        let topic = store.create("Current", false, &trace).unwrap();
        store.set_automatic(false, &trace).unwrap();

        assert!(matches!(
            store
                .clone()
                .while_automatic()
                .rename(&topic, "Late model rename", &trace),
            Err(TopicError::AutomaticDisabled)
        ));
        assert_eq!(store.get(&topic).unwrap().title, "Current");
    }

    #[test]
    fn human_corrections_can_remove_archived_membership_but_not_add_it() {
        let (store, turn, trace) = setup();
        let archived = store.create("Archived", false, &trace).unwrap();
        let active = store.create("Active", false, &trace).unwrap();
        store
            .assign_model(
                &turn,
                "draft",
                vec![archived.clone(), active.clone()],
                &trace,
            )
            .unwrap();
        store.archive(&archived, &trace).unwrap();

        store
            .assign_human(&turn, "draft", vec![active.clone()], &trace)
            .unwrap();
        assert!(matches!(
            store.assign_human(&turn, "new outcome", vec![archived], &trace),
            Err(TopicError::Store(_))
        ));
        let corrected = store.snapshot().unwrap();
        assert_eq!(corrected.placements[0].topic_ids, vec![active]);
    }

    #[test]
    fn each_conversation_has_a_sealed_document_and_corruption_fails_closed() {
        let (store, _, trace) = setup();
        let session = store.session_id.clone();
        let scope = vak_config::scope::AgentScope::new(vak_config::paths::agent_home_at(
            &vak_config::paths::data_home(),
            "topics-test-agent",
        ));
        store.create("Private topic title", false, &trace).unwrap();
        let stored = crate::documents::read(&store.path).unwrap().unwrap();
        assert!(!stored.contains("Private topic title"));

        let another = TopicStore::new(&scope, &Uuid::now_v7().hyphenated().to_string()).unwrap();
        assert!(another.snapshot().unwrap().topics.is_empty());

        let bad_session = Uuid::now_v7().hyphenated().to_string();
        let bad_store = TopicStore::new(&scope, &bad_session).unwrap();
        crate::documents::create_sealed(
            &bad_store.path,
            &crate::objects::conversation_scope(&bad_session),
            "not a topic snapshot",
        )
        .unwrap();
        assert!(matches!(bad_store.snapshot(), Err(TopicError::Invalid(_))));
        assert_eq!(store.session_id, session);
    }

    #[test]
    fn topic_ids_are_full_uuidv7_and_snapshots_preserve_mutation_provenance() {
        let (store, _, trace) = setup();
        let id = store.create("A topic", false, &trace).unwrap();
        let uuid = Uuid::parse_str(id.as_str().strip_prefix("top_").unwrap()).unwrap();
        assert_eq!(uuid.get_version_num(), 7);
        let snapshot = store.snapshot().unwrap();
        assert_eq!(snapshot.revision, 1);
        let mutation = snapshot.last_mutation.unwrap();
        assert_eq!(mutation.kind, MutationKind::Created);
        assert_eq!(mutation.actor, trace::local::local_owner());
        assert_eq!(
            mutation.trace.session_id().as_deref(),
            Some(store.session_id.as_str())
        );
    }

    #[test]
    fn whole_turn_human_assignment_protects_all_outcomes_from_model_updates() {
        let (store, turn, trace) = setup();
        let topic = store.create("Chosen by person", false, &trace).unwrap();
        assert!(
            store
                .assign_turn_human(&turn, vec![topic.clone()], &trace)
                .unwrap()
        );
        assert!(
            !store
                .assign_model(&turn, "a different outcome", vec![], &trace)
                .unwrap()
        );
        let snapshot = store.snapshot().unwrap();
        assert_eq!(snapshot.placements.len(), 1);
        assert!(snapshot.placements[0].whole_turn);
        assert_eq!(snapshot.placements[0].topic_ids, vec![topic]);
    }

    #[test]
    fn create_for_outcomes_commits_topic_and_memberships_in_one_document_version() {
        let (store, turn, trace) = setup();
        let outcomes = vec!["Research the source".into(), "Draft the reply".into()];

        let id = store
            .clone()
            .while_automatic()
            .create_for_outcomes("Source review", true, &turn, &outcomes, &trace)
            .unwrap();

        let snapshot = store.snapshot().unwrap();
        assert_eq!(snapshot.revision, 1);
        assert_eq!(crate::documents::version_count(&store.path), 1);
        assert_eq!(snapshot.topics.len(), 1);
        assert_eq!(snapshot.topics[0].id, id);
        assert!(snapshot.topics[0].pinned);
        assert!(snapshot.placements.iter().all(|placement| {
            placement.topic_ids == [id.clone()]
                && placement.source == PlacementSource::Model
                && placement.turn_id == turn
                && placement.trace == trace
        }));
        let actual_outcomes = snapshot
            .placements
            .iter()
            .map(|placement| placement.outcome.as_str())
            .collect::<std::collections::HashSet<_>>();
        let expected_outcomes = ["Research the source", "Draft the reply"]
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(actual_outcomes, expected_outcomes);
        let mutation = snapshot.last_mutation.unwrap();
        assert_eq!(mutation.kind, MutationKind::Created);
        assert_eq!(mutation.target.as_deref(), Some(id.as_str()));
        assert_eq!(mutation.trace, trace);
    }

    #[test]
    fn create_for_outcomes_refuses_human_turn_or_exact_outcome_corrections_without_orphans() {
        let outcomes = vec!["Research".into(), "Draft".into()];

        let (whole_store, whole_turn, whole_trace) = setup();
        let chosen = whole_store
            .create("Human choice", false, &whole_trace)
            .unwrap();
        whole_store
            .assign_turn_human(&whole_turn, vec![chosen], &whole_trace)
            .unwrap();
        let before = whole_store.snapshot().unwrap();
        assert!(matches!(
            whole_store.create_for_outcomes(
                "Must not be created",
                false,
                &whole_turn,
                &outcomes,
                &whole_trace
            ),
            Err(TopicError::Invalid(_))
        ));
        let after = whole_store.snapshot().unwrap();
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.topics.len(), before.topics.len());

        let (outcome_store, outcome_turn, outcome_trace) = setup();
        outcome_store
            .assign_human(&outcome_turn, "Research", Vec::new(), &outcome_trace)
            .unwrap();
        let before = outcome_store.snapshot().unwrap();
        assert!(matches!(
            outcome_store.create_for_outcomes(
                "Must not be created",
                false,
                &outcome_turn,
                &outcomes,
                &outcome_trace
            ),
            Err(TopicError::Invalid(_))
        ));
        let after = outcome_store.snapshot().unwrap();
        assert_eq!(after.revision, before.revision);
        assert!(after.topics.is_empty());
        assert_eq!(after.placements.len(), 1);
        assert!(after.placements[0].topic_ids.is_empty());
    }

    #[test]
    fn create_for_outcomes_rechecks_automatic_mode_and_never_reuses_archived_topic() {
        let (store, turn, trace) = setup();
        let outcomes = vec!["Follow-up".into()];
        let archived = store
            .create_for_outcomes("Continuing work", false, &turn, &outcomes, &trace)
            .unwrap();
        store.archive(&archived, &trace).unwrap();

        let before_disabled = store.snapshot().unwrap();
        store.set_automatic(false, &trace).unwrap();
        let before_attempt = store.snapshot().unwrap();
        assert!(matches!(
            store.clone().while_automatic().create_for_outcomes(
                "Continuing work",
                false,
                &turn,
                &["Another outcome".into()],
                &trace
            ),
            Err(TopicError::AutomaticDisabled)
        ));
        let disabled = store.snapshot().unwrap();
        assert_eq!(disabled.topics.len(), before_disabled.topics.len());
        assert_eq!(disabled.revision, before_attempt.revision);

        store.set_automatic(true, &trace).unwrap();
        let fresh = store
            .create_for_outcomes("Continuing work", false, &turn, &outcomes, &trace)
            .unwrap();
        assert_ne!(fresh, archived);
        assert!(store.get(&archived).unwrap().archived);
        assert!(!store.get(&fresh).unwrap().archived);
        let snapshot = store.snapshot().unwrap();
        let current = snapshot
            .placements
            .iter()
            .find(|placement| placement.outcome == "Follow-up")
            .unwrap();
        assert_eq!(current.topic_ids, vec![fresh]);
    }
}
