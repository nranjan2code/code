use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use vak_llm::Message;

use crate::turns::{Evidence, Fidelity, TurnCard, TurnIndex, WorkingSetPlan};
use crate::types::{
    CompactionPlan, Entry, EntryPayload, MessageMeta, MessageRecord, PresentationRecord,
    SessionError, SessionHeader, TranscriptMessage, TurnCardRecord, WorkEvent,
};

/// Session-derived content for the request tail (docs/design/68-context-
/// engine.md §6/§10), rendered by the caller alongside the host-supplied
/// temporal/stance content instead of being spliced into `derive_messages`.
/// `None` means there is nothing to say for that section this turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TailSections {
    pub intent: Option<String>,
    pub work_contract: Option<String>,
    pub thread: Option<String>,
}

pub struct SessionLog {
    path: PathBuf,
    file: File,
    read_only: bool,
    entries: Vec<Entry>,
    by_id: HashMap<String, usize>,
    tail_id: Option<String>,
    tail_hash: Option<String>,
    warnings: Vec<String>,
}

impl SessionLog {
    /// Returns whether this append-only ledger already recorded admission for
    /// a client request. This is used to make network retries idempotent.
    pub fn has_request_admission(&self, request_id: &str) -> bool {
        self.entries.iter().any(|entry| {
            matches!(&entry.payload, EntryPayload::Activity(activity)
                if activity.data.get("request_id").map(String::as_str) == Some(request_id))
        })
    }
    pub fn create(path: PathBuf, header: SessionHeader) -> Result<Self, SessionError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .truncate(false)
            .open(&path)?;
        // Cross-process safety: an exclusive lock for the lifetime of the
        // handle keeps two processes from interleaving appends.
        file.try_lock()
            .map_err(|_| SessionError::Locked(path.clone()))?;
        if file.metadata()?.len() > 0 {
            return Err(SessionError::Exists(path));
        }
        let mut log = SessionLog {
            path,
            file,
            read_only: false,
            entries: Vec::new(),
            by_id: HashMap::new(),
            tail_id: None,
            tail_hash: None,
            warnings: Vec::new(),
        };
        log.append(Entry::new(None, EntryPayload::Header(header)))?;
        Ok(log)
    }
}

struct ParsedEntries {
    entries: Vec<Entry>,
    by_id: HashMap<String, usize>,
    tail_id: Option<String>,
    tail_hash: Option<String>,
    warnings: Vec<String>,
}

impl SessionLog {
    fn parse_entries(path: &Path, reader: BufReader<File>) -> Result<ParsedEntries, SessionError> {
        let mut entries = Vec::new();
        let mut by_id = HashMap::new();
        let mut warnings = Vec::new();
        let mut expected_prev: Option<String> = None;
        let mut tail_hash: Option<String> = None;
        let mut unchained = 0usize;
        let mut broken = Vec::new();
        for (i, line) in reader.lines().enumerate() {
            let line = line.map_err(|e| SessionError::Corrupt {
                line: i + 1,
                message: e.to_string(),
            })?;
            if line.trim().is_empty() {
                continue;
            }
            // A torn final line (crash mid-append) or a damaged interior
            // line must not make the whole session unresumable; skip it
            // and surface a warning. The append-only ledger on disk is
            // never rewritten.
            let Ok(entry) = serde_json::from_str::<Entry>(&line) else {
                warnings.push(format!(
                    "skipped unparseable entry at line {} of {}",
                    i + 1,
                    path.display()
                ));
                expected_prev = None;
                continue;
            };
            match (&entry.prev_hash, &expected_prev) {
                // An entry that carries a link must match it. A mismatch means
                // the ledger was edited after the fact, and that is reported
                // rather than raised: the record is evidence, and refusing to
                // open it would destroy the only copy of what happened.
                (Some(found), Some(want)) if found != want => broken.push(i + 1),
                // Absent where a predecessor exists: written before chaining.
                // The very first entry legitimately has no link.
                (None, Some(_)) => unchained += 1,
                _ => {}
            }
            let digest = crate::types::line_digest(&line);
            expected_prev = Some(digest.clone());
            tail_hash = Some(digest);
            by_id.insert(entry.id.clone(), entries.len());
            entries.push(entry);
        }
        if !broken.is_empty() {
            warnings.push(format!(
                "ledger {} has a broken hash chain at line(s) {}: \
                 entries before that point were modified after they were written",
                path.display(),
                broken
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if unchained > 0 {
            warnings.push(format!(
                "ledger {} has {unchained} entry/entries written before hash chaining; \
                 those cannot be verified",
                path.display()
            ));
        }
        let tail_id = entries.last().map(|e| e.id.clone());
        Ok(ParsedEntries {
            entries,
            by_id,
            tail_id,
            tail_hash,
            warnings,
        })
    }

    pub fn open(path: PathBuf) -> Result<Self, SessionError> {
        let file = OpenOptions::new().append(true).open(&path)?;
        file.try_lock()
            .map_err(|_| SessionError::Locked(path.clone()))?;
        let reader = BufReader::new(File::open(&path)?);
        let parsed = Self::parse_entries(&path, reader)?;
        Ok(SessionLog {
            path,
            file,
            read_only: false,
            entries: parsed.entries,
            by_id: parsed.by_id,
            tail_id: parsed.tail_id,
            tail_hash: parsed.tail_hash,
            warnings: parsed.warnings,
        })
    }

    /// Open an existing session for reading and inspection without acquiring an
    /// exclusive write lock. Allows web clients, exports, and inspectors to
    /// read and rehydrate sessions that are currently active in another process.
    pub fn open_read_only(path: PathBuf) -> Result<Self, SessionError> {
        let file = File::open(&path)?;
        let reader = BufReader::new(File::open(&path)?);
        let parsed = Self::parse_entries(&path, reader)?;
        Ok(SessionLog {
            path,
            file,
            read_only: true,
            entries: parsed.entries,
            by_id: parsed.by_id,
            tail_id: parsed.tail_id,
            tail_hash: parsed.tail_hash,
            warnings: parsed.warnings,
        })
    }

    /// Returns whether this SessionLog was opened read-only.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Non-fatal problems seen while opening the ledger.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn append(&mut self, entry: Entry) -> Result<Entry, SessionError> {
        if self.read_only {
            return Err(SessionError::Locked(self.path.clone()));
        }
        if let Some(pid) = &entry.parent_id
            && !self.by_id.contains_key(pid)
        {
            return Err(SessionError::Corrupt {
                line: 0,
                message: format!("parent entry {pid} not found"),
            });
        }
        let mut entry = entry;
        entry.prev_hash = self.tail_hash.clone();
        let line = serde_json::to_string(&entry).map_err(|e| SessionError::Corrupt {
            line: 0,
            message: e.to_string(),
        })?;
        writeln!(self.file, "{line}")?;
        // `File::flush` is a no-op — `std::fs::File` has no userspace buffer,
        // so its `Write::flush` returns Ok without a syscall. That is what
        // this used to call, which meant the "durable, reconstructable"
        // ledger had no write barrier at all and lost its tail on power loss.
        // `sync_data` skips the metadata flush `sync_all` forces; the file
        // length is data for an append-only log.
        self.file.sync_data()?;
        self.tail_hash = Some(crate::types::line_digest(&line));
        self.by_id.insert(entry.id.clone(), self.entries.len());
        self.tail_id = Some(entry.id.clone());
        self.entries.push(entry.clone());
        Ok(entry)
    }

    pub fn append_message(&mut self, record: MessageRecord) -> Result<Entry, SessionError> {
        let parent = self.tail_id.clone();
        self.append(Entry::new(parent, EntryPayload::Message(record)))
    }

    /// Appends a work receipt (audit entry; never model-visible).
    pub fn append_receipt(&mut self, receipt: vak_llm::WorkReceipt) -> Result<Entry, SessionError> {
        let parent = self.tail_id.clone();
        self.append(Entry::new(parent, EntryPayload::Receipt(receipt)))
    }

    /// Appends a goal-lifecycle entry (audit; never model-visible).
    pub fn append_goal(&mut self, goal: crate::types::GoalEntry) -> Result<Entry, SessionError> {
        let parent = self.tail_id.clone();
        self.append(Entry::new(parent, EntryPayload::Goal(goal)))
    }

    pub fn append_goal_update(
        &mut self,
        update: vak_intent::GoalUpdate,
    ) -> Result<Entry, SessionError> {
        let parent = self.tail_id.clone();
        self.append(Entry::new(parent, EntryPayload::GoalUpdate(update)))
    }

    pub fn latest_goal_update(&self) -> Option<&vak_intent::GoalUpdate> {
        self.entries
            .iter()
            .rev()
            .find_map(|entry| match &entry.payload {
                EntryPayload::GoalUpdate(update) => Some(update),
                _ => None,
            })
    }

    pub fn active_goal_revision(&self) -> Option<u64> {
        self.entries
            .iter()
            .rev()
            .find_map(|entry| match &entry.payload {
                EntryPayload::GoalUpdate(update)
                    if matches!(
                        update.relation,
                        vak_intent::GoalRelation::New
                            | vak_intent::GoalRelation::AddsTo
                            | vak_intent::GoalRelation::Corrects
                            | vak_intent::GoalRelation::Replaces
                    ) =>
                {
                    Some(update.revision)
                }
                _ => None,
            })
    }

    pub fn goal_state(&self) -> Option<vak_intent::GoalState> {
        vak_intent::GoalState::from_updates(self.chain_to_root().iter().filter_map(|entry| {
            if let EntryPayload::GoalUpdate(update) = &entry.payload {
                Some(update.clone())
            } else {
                None
            }
        }))
    }

    /// Appends a presentation/audit lifecycle fact. It is deliberately
    /// excluded from `derive_messages`.
    pub fn append_activity(
        &mut self,
        activity: crate::types::ActivityRecord,
    ) -> Result<Entry, SessionError> {
        let parent = self.tail_id.clone();
        self.append(Entry::new(parent, EntryPayload::Activity(activity)))
    }

    /// Appends a validated presentation (docs/design/68-context-engine.md
    /// §10). Hash-linked like every entry; never rewritten.
    pub fn append_presentation(
        &mut self,
        record: PresentationRecord,
    ) -> Result<Entry, SessionError> {
        let parent = self.tail_id.clone();
        self.append(Entry::new(parent, EntryPayload::Presentation(record)))
    }

    /// The most recently recorded capacity profile matching `key`,
    /// reconstructed from the ledger's `CapacityProbe`/`CapacityFeedback`
    /// activities (docs/design/68-context-engine.md §1). Generic over the
    /// caller's key/profile types (vak-agent's `ProfileKey`/`CapacityProfile`)
    /// because vak-session cannot depend on vak-agent — that dependency runs
    /// the other way — so this crate stores and retrieves the profile as the
    /// JSON the caller already serialized, never interpreting its shape.
    pub fn latest_capacity_profile<K, P>(&self, key: &K) -> Option<P>
    where
        K: serde::Serialize,
        P: serde::de::DeserializeOwned,
    {
        let key_json = serde_json::to_value(key).ok()?;
        self.chain_to_root().into_iter().rev().find_map(|entry| {
            let EntryPayload::Activity(activity) = &entry.payload else {
                return None;
            };
            if !matches!(
                activity.kind,
                crate::types::ActivityKind::CapacityProbe
                    | crate::types::ActivityKind::CapacityFeedback
            ) {
                return None;
            }
            let stored_key: serde_json::Value =
                serde_json::from_str(activity.data.get("key")?).ok()?;
            if stored_key != key_json {
                return None;
            }
            serde_json::from_str(activity.data.get("profile")?).ok()
        })
    }

    /// Appends a turn's closing card (docs/design/68-context-engine.md §10).
    /// Written once, at turn close, and never rewritten.
    pub fn append_turn_card(&mut self, record: TurnCardRecord) -> Result<Entry, SessionError> {
        let parent = self.tail_id.clone();
        self.append(Entry::new(parent, EntryPayload::TurnCard(record)))
    }

    /// `(turn_id, card)` for every `TurnCard` entry along the active chain,
    /// in the order they were written.
    pub fn turn_cards(&self) -> Vec<(String, TurnCard)> {
        self.chain_to_root()
            .into_iter()
            .filter_map(|entry| match &entry.payload {
                EntryPayload::TurnCard(record) => {
                    Some((record.turn_id.clone(), record.card.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// Resolves a tool_use_id to its full `Evidence` — the tool name, its
    /// call arguments, its result content, and whether it failed — by
    /// scanning the active chain for the matching `ToolUse`/`ToolResult`
    /// pair. Works for evidence in any turn, open or closed, which is what
    /// lets `recall({ id })` reopen a result from a turn now reduced to a
    /// card (docs/design/68-context-engine.md §3).
    pub fn evidence(&self, tool_use_id: &str) -> Option<Evidence> {
        let chain = self.chain_to_root();
        let mut tool: Option<String> = None;
        let mut input = serde_json::Value::Null;
        for entry in &chain {
            let EntryPayload::Message(record) = &entry.payload else {
                continue;
            };
            for block in &record.message.content {
                if let vak_llm::ContentBlock::ToolUse {
                    id,
                    name,
                    input: call_input,
                } = block
                    && id == tool_use_id
                {
                    tool = Some(name.clone());
                    input = call_input.clone();
                }
            }
        }
        let tool = tool?;
        for entry in &chain {
            let EntryPayload::Message(record) = &entry.payload else {
                continue;
            };
            for block in &record.message.content {
                if let vak_llm::ContentBlock::ToolResult {
                    tool_use_id: id,
                    content,
                    is_error,
                } = block
                    && id == tool_use_id
                {
                    return Some(Evidence {
                        tool,
                        input,
                        content: content.clone(),
                        is_error: *is_error,
                    });
                }
            }
        }
        None
    }

    /// Presentation entries along the active path, root→leaf, with their
    /// entry ids — the single source both the model-visible history and the
    /// display channel read.
    pub fn presentations(&self) -> Vec<(String, &PresentationRecord)> {
        self.chain_to_root()
            .into_iter()
            .filter_map(|e| match &e.payload {
                EntryPayload::Presentation(record) => Some((e.id.clone(), record)),
                _ => None,
            })
            .collect()
    }

    /// Whether a presentation with this exact canonical payload already
    /// exists for this turn — the duplicate rule that drops a repeated
    /// fence from the model-visible projection (docs/design/68-context-engine.md
    /// §10: "a second card in the same turn with the same `payload_digest`
    /// is not written").
    pub fn has_presentation(&self, turn_id: &str, payload_digest: &str) -> bool {
        self.presentations()
            .iter()
            .any(|(_, record)| record.turn_id == turn_id && record.payload_digest == payload_digest)
    }

    /// The id of the most recent non-control user message entry in the
    /// active chain — the directive the current turn answers. `None` before
    /// any real user message exists.
    pub fn latest_directive_entry_id(&self) -> Option<String> {
        self.chain_to_root()
            .into_iter()
            .rev()
            .find_map(|entry| match &entry.payload {
                EntryPayload::Message(record)
                    if record.message.role == vak_llm::Role::User
                        && record.control_kind().is_none()
                        && record
                            .message
                            .content
                            .iter()
                            .any(|b| matches!(b, vak_llm::ContentBlock::Text { .. }))
                        && !record
                            .message
                            .content
                            .iter()
                            .any(|b| matches!(b, vak_llm::ContentBlock::ToolResult { .. })) =>
                {
                    Some(entry.id.clone())
                }
                _ => None,
            })
    }

    /// Ids of every non-card tool result already committed to the ledger
    /// strictly after `turn_id`, in chain order — the evidence a card built
    /// after them was derived from (`PresentationRecord::derived_from`).
    /// `is_card_tool` is supplied by the caller so this crate never needs to
    /// know what a "card" tool is (that knowledge lives in
    /// `vak-core::presentation_tools`).
    pub fn non_card_evidence_since(
        &self,
        turn_id: &str,
        is_card_tool: impl Fn(&str) -> bool,
    ) -> Vec<String> {
        let chain = self.chain_to_root();
        let start = chain
            .iter()
            .position(|entry| entry.id == turn_id)
            .map(|idx| idx + 1)
            .unwrap_or(0);
        let mut tool_names: HashMap<String, String> = HashMap::new();
        let mut out = Vec::new();
        for entry in &chain[start..] {
            let EntryPayload::Message(record) = &entry.payload else {
                continue;
            };
            for block in &record.message.content {
                match block {
                    vak_llm::ContentBlock::ToolUse { id, name, .. } => {
                        tool_names.insert(id.clone(), name.clone());
                    }
                    vak_llm::ContentBlock::ToolResult { tool_use_id, .. } => {
                        let is_card = tool_names
                            .get(tool_use_id)
                            .map(|name| is_card_tool(name))
                            .unwrap_or(false);
                        if !is_card && !out.iter().any(|seen| seen == tool_use_id) {
                            out.push(tool_use_id.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
        out
    }

    /// Append a finalized or provisional voice transcript. The transcript
    /// is an audit projection; callers must append a normal Message entry
    /// separately when the utterance is committed as model input.
    pub fn append_voice_transcript(
        &mut self,
        activity_id: impl Into<String>,
        text: impl Into<String>,
        finalized: bool,
    ) -> Result<Entry, SessionError> {
        let mut data = std::collections::BTreeMap::new();
        data.insert("text".into(), text.into());
        data.insert("finalized".into(), finalized.to_string());
        self.append_activity(crate::types::ActivityRecord {
            activity_id: activity_id.into(),
            turn: None,
            kind: crate::types::ActivityKind::VoiceTranscript,
            status: if finalized {
                crate::types::ActivityStatus::Succeeded
            } else {
                crate::types::ActivityStatus::Running
            },
            label: "Voice transcript".into(),
            detail: None,
            data,
        })
    }

    /// Append an auditable presentation selection without making it part of
    /// model context. The original result and fallback remain authoritative.
    pub fn append_presentation_selection(
        &mut self,
        activity_id: impl Into<String>,
        semantic_type: impl Into<String>,
        spec_id: impl Into<String>,
        revision: u64,
        mode: impl Into<String>,
        fallback_used: bool,
    ) -> Result<Entry, SessionError> {
        let mut data = std::collections::BTreeMap::new();
        data.insert("semantic_type".into(), semantic_type.into());
        data.insert("spec_id".into(), spec_id.into());
        data.insert("revision".into(), revision.to_string());
        data.insert("mode".into(), mode.into());
        data.insert("fallback_used".into(), fallback_used.to_string());
        self.append_activity(crate::types::ActivityRecord {
            activity_id: activity_id.into(),
            turn: None,
            kind: crate::types::ActivityKind::PresentationSelection,
            status: crate::types::ActivityStatus::Succeeded,
            label: "Presentation selected".into(),
            detail: None,
            data,
        })
    }

    /// Append a projection choice or correction. If the text is sent to the
    /// model, callers must also append a normal Message entry so derivation
    /// remains complete.
    pub fn append_presentation_feedback(
        &mut self,
        activity_id: impl Into<String>,
        choice: impl Into<String>,
        feedback: Option<String>,
    ) -> Result<Entry, SessionError> {
        let mut data = std::collections::BTreeMap::new();
        data.insert("choice".into(), choice.into());
        if let Some(feedback) = feedback {
            data.insert("feedback".into(), feedback);
        }
        self.append_activity(crate::types::ActivityRecord {
            activity_id: activity_id.into(),
            turn: None,
            kind: crate::types::ActivityKind::PresentationFeedback,
            status: crate::types::ActivityStatus::Succeeded,
            label: "Presentation feedback".into(),
            detail: None,
            data,
        })
    }

    /// Append what the playback client reports it emitted. This is distinct
    /// from provider output because buffering and interruption can prevent
    /// the user from hearing the complete synthesis.
    pub fn append_voice_playback(
        &mut self,
        activity_id: impl Into<String>,
        emitted_ms: u64,
        interrupted: bool,
    ) -> Result<Entry, SessionError> {
        let mut data = std::collections::BTreeMap::new();
        data.insert("emitted_ms".into(), emitted_ms.to_string());
        data.insert("interrupted".into(), interrupted.to_string());
        self.append_activity(crate::types::ActivityRecord {
            activity_id: activity_id.into(),
            turn: None,
            kind: crate::types::ActivityKind::VoicePlayback,
            status: if interrupted {
                crate::types::ActivityStatus::Partial
            } else {
                crate::types::ActivityStatus::Succeeded
            },
            label: "Voice playback".into(),
            detail: None,
            data,
        })
    }

    /// Record this turn's resolved intent.
    ///
    /// Appended before the turn dispatches, so the note it carries is in the
    /// projection the model actually sees — the entry *is* the record of what
    /// was said, not a description of it (invariant 1).
    pub fn append_intent(
        &mut self,
        record: crate::types::IntentRecord,
    ) -> Result<Entry, SessionError> {
        let parent = self.tail_id.clone();
        self.append(Entry::new(parent, EntryPayload::Intent(Box::new(record))))
    }

    pub fn append_child_run_status(
        &mut self,
        status: crate::types::ChildRunStatus,
    ) -> Result<Entry, SessionError> {
        self.append_child_run_result(status, None)
    }

    pub fn append_child_run_result(
        &mut self,
        status: crate::types::ChildRunStatus,
        outcome: Option<vak_intent::OutcomeSpec>,
    ) -> Result<Entry, SessionError> {
        let parent = self.tail_id.clone();
        self.append(Entry::new(
            parent,
            EntryPayload::ChildRun { status, outcome },
        ))
    }

    pub fn child_run_status(&self) -> Option<crate::types::ChildRunStatus> {
        self.chain_to_root()
            .iter()
            .rev()
            .find_map(|entry| match &entry.payload {
                EntryPayload::ChildRun { status, .. } => Some(status.clone()),
                _ => None,
            })
    }

    pub fn child_run_outcome(&self) -> Option<vak_intent::OutcomeSpec> {
        self.chain_to_root()
            .iter()
            .rev()
            .find_map(|entry| match &entry.payload {
                EntryPayload::ChildRun { outcome, .. } => outcome.clone(),
                _ => None,
            })
    }

    pub fn append_turn_capabilities(
        &mut self,
        bound: crate::types::TurnCapabilitiesBound,
    ) -> Result<Entry, SessionError> {
        let parent = self.tail_id.clone();
        self.append(Entry::new(
            parent,
            EntryPayload::TurnCapabilitiesBound(bound),
        ))
    }

    pub fn append_work(&mut self, event: WorkEvent) -> Result<Entry, SessionError> {
        let parent = self.tail_id.clone();
        let candidate = Entry::new(parent, EntryPayload::Work(event));
        let mut chain = self.chain_to_root();
        chain.push(&candidate);
        crate::work::project_work(&chain).map_err(|error| SessionError::Corrupt {
            line: 0,
            message: format!("invalid work event: {error}"),
        })?;
        let appended = self.append(candidate)?;
        self.promote_ready_work_items()?;
        Ok(appended)
    }

    fn promote_ready_work_items(&mut self) -> Result<(), SessionError> {
        let Some(projection) = self
            .work_projection()
            .map_err(|error| SessionError::Corrupt {
                line: 0,
                message: error.to_string(),
            })?
        else {
            return Ok(());
        };
        if projection.status != crate::types::WorkContractStatus::Active {
            return Ok(());
        }
        let ready: Vec<String> = projection
            .contract
            .items
            .iter()
            .filter(|definition| {
                projection
                    .items
                    .get(&definition.item_id)
                    .is_some_and(|state| state.status == crate::types::WorkItemStatus::Proposed)
                    && definition.dependencies.iter().all(|dependency| {
                        projection.items.get(dependency).is_some_and(|state| {
                            matches!(
                                state.status,
                                crate::types::WorkItemStatus::Succeeded
                                    | crate::types::WorkItemStatus::Skipped
                            )
                        })
                    })
            })
            .map(|definition| definition.item_id.clone())
            .collect();
        for item_id in ready {
            let current = self
                .work_projection()
                .map_err(|error| SessionError::Corrupt {
                    line: 0,
                    message: error.to_string(),
                })?;
            let Some(current) = current else { break };
            self.append_work(crate::types::WorkEvent {
                contract_id: current.contract.contract_id,
                revision: current.contract.revision,
                kind: crate::types::WorkEventKind::ItemStatusChanged {
                    item_id,
                    from: crate::types::WorkItemStatus::Proposed,
                    to: crate::types::WorkItemStatus::Ready,
                    attempt: 0,
                    reason: "dependencies satisfied".into(),
                },
            })?;
        }
        Ok(())
    }

    pub fn work_projection(
        &self,
    ) -> Result<Option<crate::work::WorkProjection>, crate::work::WorkError> {
        crate::work::project_work(&self.chain_to_root())
    }

    pub fn evidence_exists(&self, evidence: &crate::types::EvidenceRef) -> bool {
        let session_id = self.header().map(|header| header.session_id.as_str());
        self.chain_to_root().iter().any(|entry| match evidence {
            crate::types::EvidenceRef::LedgerEntry {
                session_id: evidence_session,
                entry_id,
            }
            | crate::types::EvidenceRef::Receipt {
                session_id: evidence_session,
                entry_id,
            } => session_id == Some(evidence_session.as_str()) && entry.id == *entry_id,
            crate::types::EvidenceRef::ToolResult {
                session_id: evidence_session,
                tool_use_id,
            } => {
                session_id == Some(evidence_session.as_str())
                    && matches!(
                        &entry.payload,
                        crate::types::EntryPayload::Message(record)
                            if record.message.content.iter().any(|block| matches!(
                                block,
                                vak_llm::ContentBlock::ToolResult { tool_use_id: id, .. }
                                    if id == tool_use_id
                            ))
                    )
            }
            crate::types::EvidenceRef::CheckpointDiff { .. }
            | crate::types::EvidenceRef::FlowNode { .. }
            | crate::types::EvidenceRef::ChildSession { .. }
            | crate::types::EvidenceRef::ExternalOperation { .. } => false,
        })
    }

    /// Reconcile managed items left running by a process restart. Only child
    /// sessions explicitly reported as live may remain running; every other
    /// running item is conservatively interrupted and made retry-eligible.
    /// Possible side effects are never replayed automatically.
    pub fn reconcile_running_work(
        &mut self,
        live_child_sessions: &std::collections::HashSet<String>,
    ) -> Result<usize, SessionError> {
        self.reconcile_running_work_with_child_ledgers(live_child_sessions, None)
    }

    pub fn reconcile_running_work_with_child_ledgers(
        &mut self,
        live_child_sessions: &std::collections::HashSet<String>,
        sessions_home: Option<&Path>,
    ) -> Result<usize, SessionError> {
        let Some(projection) = self
            .work_projection()
            .map_err(|error| SessionError::Corrupt {
                line: 0,
                message: error.to_string(),
            })?
        else {
            return Ok(0);
        };
        let stale: Vec<(String, u32, Option<String>)> = projection
            .items
            .values()
            .filter(|item| {
                item.status == crate::types::WorkItemStatus::Running
                    && !item
                        .child_session_id
                        .as_ref()
                        .is_some_and(|id| live_child_sessions.contains(id))
            })
            .map(|item| {
                (
                    item.item_id.clone(),
                    item.attempt,
                    item.child_session_id.clone(),
                )
            })
            .collect();
        let mut reconciled = 0;
        for (item_id, attempt, child_id) in stale {
            let Some(current) = self
                .work_projection()
                .map_err(|error| SessionError::Corrupt {
                    line: 0,
                    message: error.to_string(),
                })?
            else {
                break;
            };
            let child_status = child_id.as_deref().and_then(|id| {
                let home = sessions_home?;
                let cwd = self.header().map(|h| h.contract_cwd())?;
                let path = SessionPath::new_session_file(home, &cwd, id);
                SessionLog::open(path)
                    .ok()
                    .and_then(|child| child.child_run_status())
            });
            if matches!(child_status, Some(crate::types::ChildRunStatus::Completed)) {
                self.append_work(WorkEvent {
                    contract_id: current.contract.contract_id.clone(),
                    revision: current.contract.revision,
                    kind: crate::types::WorkEventKind::EvidenceAttached {
                        item_id: item_id.clone(),
                        evidence: crate::types::EvidenceRef::ChildSession {
                            session_id: child_id.clone().unwrap_or_default(),
                        },
                    },
                })?;
                self.append_work(WorkEvent {
                    contract_id: current.contract.contract_id.clone(),
                    revision: current.contract.revision,
                    kind: crate::types::WorkEventKind::ItemStatusChanged {
                        item_id,
                        from: crate::types::WorkItemStatus::Running,
                        to: crate::types::WorkItemStatus::ReadyForVerification,
                        attempt,
                        reason: "recovered completed child; verify its durable evidence".into(),
                    },
                })?;
                reconciled += 1;
                continue;
            }
            self.append_work(WorkEvent {
                contract_id: current.contract.contract_id.clone(),
                revision: current.contract.revision,
                kind: crate::types::WorkEventKind::ItemStatusChanged {
                    item_id,
                    from: crate::types::WorkItemStatus::Running,
                    to: crate::types::WorkItemStatus::Interrupted,
                    attempt,
                    reason: match child_status {
                        Some(crate::types::ChildRunStatus::Failed) => {
                            "child failed before restart; review before retry"
                        }
                        Some(crate::types::ChildRunStatus::Aborted) => {
                            "child was aborted before restart; review before retry"
                        }
                        Some(crate::types::ChildRunStatus::MaxTurns) => {
                            "child hit its turn limit; review before retry"
                        }
                        _ => "recovered after process restart; review before retry",
                    }
                    .into(),
                },
            })?;
            reconciled += 1;
        }
        Ok(reconciled)
    }

    pub fn activities(
        &self,
    ) -> Vec<(
        String,
        chrono::DateTime<chrono::Utc>,
        crate::types::ActivityRecord,
    )> {
        self.chain_to_root()
            .into_iter()
            .filter_map(|entry| match &entry.payload {
                EntryPayload::Activity(activity) => {
                    Some((entry.id.clone(), entry.ts, activity.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// Successful tool-result receipts with the ledger timestamp at which
    /// the result was recorded. This is a replay-safe source for evidence
    /// freshness; callers choose the domain-specific validity window.
    pub fn successful_tool_receipts(&self) -> Vec<(String, chrono::DateTime<chrono::Utc>)> {
        let mut known_calls = std::collections::HashSet::new();
        let mut receipts = Vec::new();
        for entry in self.chain_to_root() {
            if let EntryPayload::Message(record) = &entry.payload {
                for block in &record.message.content {
                    match block {
                        vak_llm::ContentBlock::ToolUse { id, .. } => {
                            known_calls.insert(id.clone());
                        }
                        vak_llm::ContentBlock::ToolResult {
                            tool_use_id,
                            is_error: false,
                            ..
                        } if known_calls.contains(tool_use_id) => {
                            receipts.push((tool_use_id.clone(), entry.ts));
                        }
                        _ => {}
                    }
                }
            }
        }
        receipts
    }

    /// Successful receipts on the active, latest user turn only. Earlier
    /// turns are deliberately excluded so a stale unrelated command cannot
    /// establish evidence for the current result.
    pub fn successful_tool_receipts_for_latest_turn(
        &self,
    ) -> Vec<(String, chrono::DateTime<chrono::Utc>)> {
        let mut known_calls = std::collections::HashSet::new();
        let mut receipts = Vec::new();
        for entry in self.chain_to_root() {
            if let EntryPayload::Message(record) = &entry.payload {
                if record.message.role == vak_llm::Role::User
                    && record.control_kind().is_none()
                    && record
                        .message
                        .content
                        .iter()
                        .any(|block| matches!(block, vak_llm::ContentBlock::Text { .. }))
                {
                    known_calls.clear();
                    receipts.clear();
                }
                for block in &record.message.content {
                    match block {
                        vak_llm::ContentBlock::ToolUse { id, .. } => {
                            known_calls.insert(id.clone());
                        }
                        vak_llm::ContentBlock::ToolResult {
                            tool_use_id,
                            is_error: false,
                            ..
                        } if known_calls.contains(tool_use_id) => {
                            receipts.push((tool_use_id.clone(), entry.ts));
                        }
                        _ => {}
                    }
                }
            }
        }
        receipts
    }

    /// Distinct bash commands that ran GREEN on the active chain, in
    /// first-run order (docs/design/10-flows.md adoption substrate). A command is
    /// settled when its tool_result is not an error.
    pub fn settled_bash_commands(&self) -> Vec<String> {
        use std::collections::HashMap;
        // id -> is_error for tool results
        let mut results: HashMap<String, bool> = HashMap::new();
        for e in self.chain_to_root() {
            if let EntryPayload::Message(r) = &e.payload {
                for b in &r.message.content {
                    if let vak_llm::ContentBlock::ToolResult {
                        tool_use_id: id,
                        is_error,
                        ..
                    } = b
                    {
                        results.insert(id.clone(), *is_error);
                    }
                }
            }
        }
        let mut out: Vec<String> = Vec::new();
        for e in self.chain_to_root() {
            if let EntryPayload::Message(r) = &e.payload {
                for b in &r.message.content {
                    if let vak_llm::ContentBlock::ToolUse { name, input, id } = b
                        && name == "bash"
                        && !results.get(id).copied().unwrap_or(true)
                        && let Some(cmd) = input.get("command").and_then(|v| v.as_str())
                        && !out.iter().any(|o| o == cmd)
                    {
                        out.push(cmd.to_string());
                    }
                }
            }
        }
        out
    }

    /// Receipt entries along the active path, root→leaf — forensic view
    /// for surfaces that render dispatch history.
    pub fn receipts(&self) -> Vec<&vak_llm::WorkReceipt> {
        self.chain_to_root()
            .into_iter()
            .filter_map(|e| match &e.payload {
                EntryPayload::Receipt(r) => Some(r),
                _ => None,
            })
            .collect()
    }

    pub fn branch_at(&mut self, entry_id: &str) -> Result<(), SessionError> {
        if !self.by_id.contains_key(entry_id) {
            return Err(SessionError::Corrupt {
                line: 0,
                message: format!("cannot branch at unknown entry {entry_id}"),
            });
        }
        self.tail_id = Some(entry_id.to_string());
        Ok(())
    }

    pub fn compact(
        &mut self,
        summary: String,
        first_kept_entry_id: String,
        tokens_before: u64,
    ) -> Result<Entry, SessionError> {
        if !self.by_id.contains_key(&first_kept_entry_id) {
            return Err(SessionError::Corrupt {
                line: 0,
                message: format!("unknown first_kept_entry_id {first_kept_entry_id}"),
            });
        }
        let parent = self.tail_id.clone();
        self.append(Entry::new(
            parent,
            EntryPayload::Compaction(crate::types::CompactionEntry {
                summary,
                first_kept_entry_id,
                tokens_before,
                partition: None,
                reset_all: false,
            }),
        ))
    }

    /// Reset-with-handoff (docs/design/42-managed-work-contracts.md): the projection becomes ONLY
    /// this summary. Append-only; the full history stays on disk.
    pub fn append_handoff_reset(
        &mut self,
        summary: String,
        tokens_before: u64,
    ) -> Result<Entry, SessionError> {
        let parent = self.tail_id.clone();
        self.append(Entry::new(
            parent,
            EntryPayload::Compaction(crate::types::CompactionEntry {
                summary,
                first_kept_entry_id: String::new(),
                tokens_before,
                partition: None,
                reset_all: true,
            }),
        ))
    }

    pub fn header(&self) -> Option<&SessionHeader> {
        self.entries.iter().find_map(|e| match &e.payload {
            EntryPayload::Header(h) => Some(h),
            _ => None,
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn tail_id(&self) -> Option<&String> {
        self.tail_id.as_ref()
    }

    pub fn chain_to_root(&self) -> Vec<&Entry> {
        let mut chain = Vec::new();
        let mut cursor = self.tail_id.clone();
        while let Some(id) = cursor {
            let Some(&idx) = self.by_id.get(&id) else {
                break;
            };
            let entry = &self.entries[idx];
            chain.push(entry);
            cursor = entry.parent_id.clone();
        }
        chain.reverse();
        chain
    }

    /// Message entries along the active path, root→leaf, with their entry
    /// ids — raw ledger view (compaction entries NOT applied).
    pub fn message_chain(&self) -> Vec<(String, Message)> {
        self.chain_to_root()
            .into_iter()
            .filter_map(|e| match &e.payload {
                EntryPayload::Message(r) => Some((e.id.clone(), r.message.clone())),
                _ => None,
            })
            .collect()
    }

    fn derive_keyed(&self) -> Vec<(String, Message)> {
        self.derive_keyed_tagged()
            .into_iter()
            .map(|(id, m, _, _)| (id, m))
            .collect()
    }

    /// The plan-free projection: every closed turn at `Full` fidelity. Used
    /// by `derive_messages`/`plan_compaction`, which have no `CapacityProfile`
    /// to plan against.
    fn derive_keyed_tagged(&self) -> Vec<(String, Message, bool, bool)> {
        self.derive_with_plan_tagged(None)
    }

    /// Like `derive_keyed_tagged`, but a `WorkingSetPlan` (from
    /// `vak_agent::planner::plan`) selects each closed turn's fidelity
    /// instead of defaulting every one to `Full` (docs/design/68-context-
    /// engine.md §4/§10) — one implementation, the plan just picks what each
    /// turn contributes:
    ///
    /// - `Full` turns project their two-message `full_record` — no
    ///   `tool_use`/`tool_result` blocks, no character-count trim of
    ///   historical tool output (deleted: `MAX_HISTORICAL_TOOL_RESULT_CHARS`).
    /// - `Card` turns contribute one line each to a single `<turns>` block
    ///   (`TurnCard::line`), inserted once, right after any compaction
    ///   summary.
    /// - `Packet` turns, and any closed turn the plan omits entirely,
    ///   contribute nothing here: they are represented only by an existing
    ///   `Compaction` entry, which the caller (`Agent`'s incremental
    ///   compaction, §4) guarantees already covers them before this is
    ///   called with that plan.
    /// - The still-open turn (if any) always projects verbatim, regardless
    ///   of `plan` — it is never planned.
    ///
    /// The intent note, work contract, and conversation thread are rendered
    /// into the request tail instead (§6/§10), read separately via
    /// [`SessionLog::tail_sections`]; this function never contributes them.
    /// The current cumulative compaction boundary: the chain position
    /// before which everything is already summarized, and the existing
    /// summary text (if any). Shared by `derive_with_plan_tagged` and the
    /// incremental-compaction helpers below so all three agree on what
    /// "already covered" means.
    fn compaction_boundary(&self) -> (usize, HashMap<String, usize>, Option<String>) {
        let chain = self.chain_to_root();
        let position: HashMap<String, usize> = chain
            .iter()
            .enumerate()
            .map(|(i, entry)| (entry.id.clone(), i))
            .collect();
        // Each compaction's boundary is computed from the CURRENT (already
        // compacted) projection, so successive boundaries only ever move
        // forward; the last `Compaction` entry alone describes the steady
        // state.
        let last_compaction = chain
            .iter()
            .enumerate()
            .rev()
            .find_map(|(pos, entry)| match &entry.payload {
                EntryPayload::Compaction(c) => Some((pos, c.clone())),
                _ => None,
            });
        let boundary_pos = match &last_compaction {
            Some((pos, c)) if c.reset_all => *pos,
            Some((_, c)) => position
                .get(c.first_kept_entry_id.as_str())
                .copied()
                .unwrap_or(0),
            None => 0,
        };
        let summary = last_compaction.map(|(_, c)| c.summary);
        (boundary_pos, position, summary)
    }

    /// Whether the packet range ending at `last_turn_id` (a `WorkingSetPlan`
    /// packet range's newer endpoint) still needs an incremental compaction
    /// (docs/design/68-context-engine.md §4): true when no existing
    /// `Compaction` entry's boundary already reaches at or past it. `false`
    /// for an unknown id, since there is nothing to compact.
    pub fn packet_needs_compaction(&self, last_turn_id: &str) -> bool {
        let (boundary_pos, position, _) = self.compaction_boundary();
        position
            .get(last_turn_id)
            .is_some_and(|&pos| pos >= boundary_pos)
    }

    /// The summarizer input for an incremental compaction covering
    /// everything up through `last_turn_id`: the existing compaction
    /// summary (if any, so information already folded in survives) followed
    /// by one `TurnCard` line per newly covered turn — cards, not raw
    /// history (§4). Also returns a char-count estimate of that input for
    /// the caller's `tokens_before` reporting.
    pub fn packet_transcript(&self, last_turn_id: &str) -> (String, u64) {
        let (boundary_pos, position, existing_summary) = self.compaction_boundary();
        let mut out = String::new();
        if let Some(summary) = existing_summary {
            out.push_str(&summary);
            out.push_str("\n\n");
        }
        let Some(&last_pos) = position.get(last_turn_id) else {
            let chars = out.chars().count() as u64;
            return (out, chars);
        };
        let index = TurnIndex::from_log(self);
        for (turn_number, turn) in index.turns.iter().enumerate() {
            let turn_pos = position.get(turn.id.as_str()).copied().unwrap_or(0);
            if turn_pos < boundary_pos || turn_pos > last_pos {
                continue;
            }
            match &turn.card {
                Some(card) => {
                    out.push_str(&card.line(turn_number + 1));
                    out.push('\n');
                }
                None => {
                    for message in turn.full_record() {
                        out.push_str(&message.text_content());
                        out.push('\n');
                    }
                }
            }
        }
        let chars = out.chars().count() as u64;
        (out, chars)
    }

    fn derive_with_plan_tagged(
        &self,
        plan: Option<&WorkingSetPlan>,
    ) -> Vec<(String, Message, bool, bool)> {
        let (boundary_pos, position_owned, existing_summary) = self.compaction_boundary();
        let position: HashMap<&str, usize> = position_owned
            .iter()
            .map(|(id, pos)| (id.as_str(), *pos))
            .collect();

        let index = TurnIndex::from_log(self);
        let mut out: Vec<(String, Message, bool, bool)> = Vec::new();
        if let Some(summary) = &existing_summary {
            let summary_msg =
                Message::user_text(format!("<context_summary>\n{summary}\n</context_summary>"));
            out.push((String::new(), summary_msg, true, false));
        }

        let fidelity_of: HashMap<&str, Fidelity> = plan
            .map(|p| p.per_turn.iter().map(|(id, f)| (id.as_str(), *f)).collect())
            .unwrap_or_default();
        // `plan.per_turn` never carries `Fidelity::Packet` entries (§4): a
        // packeted turn is identified by falling inside `packet_range`
        // instead. Resolved once, by position, so membership is an O(1)
        // range check per turn.
        let packet_pos_range: Option<(usize, usize)> = plan.and_then(|p| {
            let (first, last) = p.packet_range.as_ref()?;
            let lo = position.get(first.as_str()).copied()?;
            let hi = position.get(last.as_str()).copied()?;
            Some((lo.min(hi), lo.max(hi)))
        });

        let mut card_lines: Vec<String> = Vec::new();
        for (turn_number, turn) in index.turns.iter().enumerate() {
            let turn_pos = position.get(turn.id.as_str()).copied().unwrap_or(0);
            if turn_pos < boundary_pos {
                continue;
            }
            if !turn.closed {
                for message in turn.current_verbatim() {
                    out.push((turn.id.clone(), message, false, false));
                }
                continue;
            }
            let selected = match plan {
                None => Fidelity::Full,
                Some(_) => {
                    if let Some(f) = fidelity_of.get(turn.id.as_str()).copied() {
                        f
                    } else if packet_pos_range
                        .is_some_and(|(lo, hi)| turn_pos >= lo && turn_pos <= hi)
                    {
                        Fidelity::Packet
                    } else if turn.card.is_some() {
                        // Has a card but the plan never classified it (a
                        // stale plan against a longer chain, or a planner
                        // bug) — render the cheap, safe form rather than
                        // silently losing it (no-cut invariant).
                        Fidelity::Card
                    } else {
                        // No card at all (a turn seeded without going
                        // through the real turn-close hook) — the only
                        // lossless choice, since there is no card to
                        // render as a line.
                        Fidelity::Full
                    }
                }
            };
            match selected {
                Fidelity::Full => {
                    for message in turn.full_record() {
                        out.push((turn.id.clone(), message, false, false));
                    }
                }
                Fidelity::Card => {
                    if let Some(card) = &turn.card {
                        card_lines.push(card.line(turn_number + 1));
                    }
                }
                Fidelity::Packet => {
                    // Represented only by an existing `Compaction` entry;
                    // the caller guarantees one covers this turn before
                    // planning with a packet range (§4).
                }
            }
        }
        if !card_lines.is_empty() {
            let block = format!("<turns>\n{}\n</turns>", card_lines.join("\n"));
            let insert_at = usize::from(existing_summary.is_some());
            out.insert(
                insert_at,
                (String::new(), Message::user_text(block), true, false),
            );
        }
        out
    }

    /// `derive_keyed_tagged` with a `WorkingSetPlan` applied — the projection
    /// the request assembler sends once a `CapacityProfile` is available
    /// (docs/design/68-context-engine.md §4/§10).
    pub fn derive_with_plan(&self, plan: &WorkingSetPlan) -> Vec<Message> {
        self.derive_with_plan_tagged(Some(plan))
            .into_iter()
            .map(|(_, m, _, _)| m)
            .collect()
    }

    /// Appends a `Compaction` entry produced by INCREMENTAL compaction (§4):
    /// summarizing the turns' CARDS (never raw history) for a packet range
    /// that a `WorkingSetPlan` just collapsed, rather than the message-level
    /// `plan_compaction`/`apply_compaction` pair used by `/compact`. The new
    /// entry's `first_kept_entry_id` is the turn immediately after
    /// `last_turn_id` — everything at or before `last_turn_id` becomes
    /// "already compacted" for every future `derive_with_plan` call, exactly
    /// like the cumulative boundary `apply_compaction` writes.
    pub fn append_incremental_compaction(
        &mut self,
        last_turn_id: &str,
        summary: String,
        tokens_before: u64,
    ) -> Result<(), SessionError> {
        let index = TurnIndex::from_log(self);
        let first_kept_entry_id = index
            .turns
            .iter()
            .skip_while(|t| t.id != last_turn_id)
            .nth(1)
            .map(|t| t.id.clone())
            .ok_or_else(|| SessionError::Corrupt {
                line: 0,
                message: format!("no turn follows packet range ending at {last_turn_id}"),
            })?;
        let parent = self.tail_id.clone();
        self.append(Entry::new(
            parent,
            EntryPayload::Compaction(crate::types::CompactionEntry {
                summary,
                first_kept_entry_id,
                tokens_before,
                partition: None,
                reset_all: false,
            }),
        ))?;
        Ok(())
    }

    /// Packet accounting for a planned turn boundary: turns before
    /// `boundary` become `dropped`, the rest stay `selected`.
    /// Every raw message entry in chain order, tagged with its ledger
    /// identity and class — a human-facing audit view, independent of the
    /// turn-based model-visible projection (`derive_messages`). Unlike that
    /// projection, this one is never affected by compaction: a person
    /// reading their own history sees everything they said, not what a
    /// context budget kept. Compaction entries still surface as a
    /// `context: true` pseudo-message so a consumer can show "context
    /// summarized here" inline.
    pub fn derive_transcript(&self) -> Vec<TranscriptMessage> {
        self.chain_to_root()
            .into_iter()
            .filter_map(|entry| match &entry.payload {
                EntryPayload::Message(record) => Some(TranscriptMessage {
                    entry_id: entry.id.clone(),
                    message: record.message.clone(),
                    control: record.control_kind(),
                    context: false,
                }),
                EntryPayload::Compaction(c) => Some(TranscriptMessage {
                    entry_id: entry.id.clone(),
                    message: Message::user_text(format!(
                        "<context_summary>\n{}\n</context_summary>",
                        c.summary
                    )),
                    control: None,
                    context: true,
                }),
                _ => None,
            })
            .collect()
    }

    /// The conversation as a person would read it: the model-visible messages
    /// minus the runtime-authored nudges. For exports and other human-facing
    /// views; the model itself is fed `derive_messages`, which is unchanged.
    pub fn derive_conversation(&self) -> Vec<Message> {
        self.derive_transcript()
            .into_iter()
            .filter(|item| item.control.is_none())
            .map(|item| item.message)
            .collect()
    }

    pub fn derive_messages(&self) -> Vec<Message> {
        self.derive_keyed().into_iter().map(|(_, m)| m).collect()
    }

    /// The three sections `derive_keyed_tagged` used to splice into the
    /// model-visible projection, computed separately for the request
    /// assembler's tail (docs/design/68-context-engine.md §6/§10). Each
    /// field is `None` when there is nothing to say — the caller renders no
    /// tag for an absent section, never an empty one.
    /// The current directive's resolved reading, distilled to a
    /// `ReadingKey` (docs/design/68-context-engine.md §4's `PlanInput`) —
    /// the newest `Intent` entry on the chain, whether or not its turn has
    /// closed yet. `None` before the first intent reading of the session.
    pub fn latest_reading(&self) -> Option<crate::turns::ReadingKey> {
        self.chain_to_root()
            .into_iter()
            .rev()
            .find_map(|entry| match &entry.payload {
                EntryPayload::Intent(record) => {
                    Some(crate::turns::ReadingKey::from_reading(&record.reading))
                }
                _ => None,
            })
    }

    pub fn tail_sections(&self) -> TailSections {
        TailSections {
            intent: self.tail_intent(),
            work_contract: self.tail_work_contract(),
            thread: self.tail_conversation_thread(),
        }
    }

    /// The latest intent note, tagged. Only the newest note applies — it
    /// otherwise wastes context and lets a stale instruction argue with the
    /// current one.
    fn tail_intent(&self) -> Option<String> {
        let note = self
            .chain_to_root()
            .into_iter()
            .rev()
            .find_map(|entry| match &entry.payload {
                EntryPayload::Intent(record) => Some(record.model_visible.clone()),
                _ => None,
            })
            .flatten()?;
        let mut block = format!("<intent>\n{note}\n</intent>");
        block.truncate(4_000);
        Some(block)
    }

    /// The active work contract's state, tagged, or `None` once it has
    /// settled (completed, failed, cancelled, or unverified).
    fn tail_work_contract(&self) -> Option<String> {
        let work = self.work_projection().ok().flatten()?;
        if matches!(
            work.status,
            crate::types::WorkContractStatus::Completed
                | crate::types::WorkContractStatus::Failed
                | crate::types::WorkContractStatus::Cancelled
                | crate::types::WorkContractStatus::Unverified
        ) {
            return None;
        }
        let mut context = format!(
            "<work_contract id=\"{}\" revision=\"{}\">\nObjective: {}\nStatus: {:?}\nItems:\n",
            work.contract.contract_id, work.contract.revision, work.contract.objective, work.status,
        );
        for item in &work.contract.items {
            if let Some(state) = work.items.get(&item.item_id) {
                context.push_str(&format!(
                    "- {}: {:?} (owner: {:?})\n",
                    item.item_id, state.status, item.owner
                ));
            }
        }
        context.push_str(
            "Rules: use this state for progress; do not claim completion before verification.\n</work_contract>",
        );
        context.truncate(4_000);
        Some(context)
    }

    /// The multi-turn directive timeline, tagged, restricted to directives
    /// whose full text is not already verbatim among `derive_messages()` —
    /// one source per fact (docs/design/68-context-engine.md §6): a
    /// directive still present in the working set verbatim needs no
    /// restating here. `None` when there is no active goal spanning
    /// multiple turns, a managed work contract already covers progress, or
    /// every directive is already verbatim in the working set.
    fn tail_conversation_thread(&self) -> Option<String> {
        let goal = self.goal_state()?;
        if !(goal.revision > 1 || !goal.additions.is_empty())
            || goal.control != vak_intent::GoalControlState::Active
            || goal.objective.trim().is_empty()
            || self.work_projection().ok().flatten().is_some()
        {
            return None;
        }

        let mut user_directives: Vec<(usize, String)> = Vec::new();
        let mut turn_counter = 1;
        for entry in self.chain_to_root() {
            if let EntryPayload::Message(record) = &entry.payload
                && record.message.role == vak_llm::Role::User
                && record.control_kind().is_none()
            {
                let text = record.message.text_content();
                let trimmed = text.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('<') {
                    user_directives.push((turn_counter, trimmed.to_string()));
                    turn_counter += 1;
                }
            }
        }
        if user_directives.len() <= 1 {
            return None;
        }

        let verbatim: std::collections::HashSet<String> = self
            .derive_messages()
            .iter()
            .map(|m| m.text_content().trim().to_string())
            .collect();
        let start_idx = user_directives.len().saturating_sub(8);
        let filtered: Vec<&(usize, String)> = user_directives[start_idx..]
            .iter()
            .filter(|(_, req)| !verbatim.contains(req.as_str()))
            .collect();
        if filtered.is_empty() {
            return None;
        }

        // Directive drift (docs/design/68-context-engine.md §7): a listed
        // directive whose reading shares no domain with the CURRENT
        // directive's is marked paused rather than dropped — a
        // re-weighting, not a cut. Turns without a card yet (never closed)
        // have no reading to compare and are never marked.
        let current_domains = self.latest_reading().map(|r| r.domains);
        let index = TurnIndex::from_log(self);

        let mut thread = format!(
            "<conversation_thread revision=\"{}\">\nPrimary objective: {}\nUser request timeline across turns:\n",
            goal.revision,
            goal.objective.trim()
        );
        for (num, req) in filtered {
            let preview = if req.len() > 200 {
                let head = req
                    .char_indices()
                    .map(|(idx, _)| idx)
                    .nth(200)
                    .unwrap_or_else(|| 200.min(req.len()));
                format!("{}...", &req[..head])
            } else {
                req.clone()
            };
            let paused = current_domains.as_ref().is_some_and(|current| {
                !current.is_empty()
                    && index.turn_by_number(*num).is_some_and(|turn| {
                        turn.card.as_ref().is_some_and(|card| {
                            !card.reading.domains.is_empty()
                                && card.reading.domains.iter().all(|d| !current.contains(d))
                        })
                    })
            });
            let suffix = if paused { " (earlier, now paused)" } else { "" };
            thread.push_str(&format!("- Turn {num}: {preview}{suffix}\n"));
        }
        thread.push_str(
            "Rules for multi-turn execution:\n\
             - Follow the user's intent across conversational drifts without complaint or friction.\n\
             - Conversational drift across turns is expected: follow along smoothly and adapt immediately.\n\
             - Resolve references (\"the data\", \"do that\", \"it\", \"something\") against the request timeline above.\n\
             - If genuinely confused, ask a brief clarification, but NEVER use asking clarification as an exception-handling escape hatch to avoid taking action or using available tools.\n\
             </conversation_thread>",
        );
        Some(thread)
    }

    /// A turn-boundary compaction plan (docs/design/68-context-engine.md
    /// §10): `older` is the full record of every closed turn being dropped;
    /// `keep_recent` is now a TURN count, not a message count — a turn is
    /// never split (principle 3), so there is no pair-boundary walk left to
    /// do. The still-open turn, if any, is never a compaction candidate: it
    /// is excluded from both `older` and the kept count.
    pub fn plan_compaction(&self, keep_recent: usize) -> Option<CompactionPlan> {
        // Operates on the CURRENT projection (already turn-based and already
        // reflecting any earlier compaction), grouped back into contiguous
        // per-turn runs — so a repeated compaction folds the prior summary
        // into the new one instead of re-reading raw pre-compaction turns,
        // and a turn is never split (its messages are always one run).
        let tagged = self.derive_keyed_tagged();
        const SUMMARY_KEY: &str = "\0summary";
        let mut units: Vec<(String, usize, usize)> = Vec::new(); // (key, start, end-exclusive)
        for (i, (id, _, is_summary, _)) in tagged.iter().enumerate() {
            let key = if *is_summary {
                SUMMARY_KEY.to_string()
            } else {
                id.clone()
            };
            match units.last_mut() {
                Some((last_key, _, end)) if *last_key == key => *end = i + 1,
                _ => units.push((key, i, i + 1)),
            }
        }
        let turn_units: Vec<&(String, usize, usize)> = units
            .iter()
            .filter(|(key, _, _)| key != SUMMARY_KEY)
            .collect();
        if turn_units.len() <= keep_recent {
            return None;
        }
        let boundary = turn_units.len() - keep_recent;
        if boundary == 0 {
            return None;
        }
        let kept_first_idx = turn_units[boundary].1;
        let older: Vec<Message> = tagged[..kept_first_idx]
            .iter()
            .map(|(_, message, _, _)| message.clone())
            .collect();
        let selected_entry_ids = turn_units[boundary..]
            .iter()
            .map(|(id, _, _)| id.clone())
            .collect();
        let dropped_entry_ids = turn_units[..boundary]
            .iter()
            .map(|(id, _, _)| id.clone())
            .collect();
        Some(CompactionPlan {
            older,
            first_kept_entry_id: turn_units[boundary].0.clone(),
            partition: crate::types::ContextPartition {
                selected_entry_ids,
                dropped_entry_ids,
            },
        })
    }

    /// Writes a compaction entry covering everything up to and including
    /// `plan.older_end_entry_id` in the *current* projection.
    pub fn apply_compaction(
        &mut self,
        plan: &CompactionPlan,
        summary: String,
        tokens_before: u64,
    ) -> Result<(), SessionError> {
        let parent = self.tail_id.clone();
        self.append(Entry::new(
            parent,
            EntryPayload::Compaction(crate::types::CompactionEntry {
                summary,
                first_kept_entry_id: plan.first_kept_entry_id.clone(),
                tokens_before,
                partition: Some(plan.partition.clone()),
                reset_all: false,
            }),
        ))?;
        Ok(())
    }

    /// Usage summed over the ACTIVE chain only: usage recorded on
    /// abandoned branches (superseded by branching/compaction) must not
    /// inflate the count.
    pub fn total_usage(&self) -> vak_llm::Usage {
        let mut total = vak_llm::Usage::default();
        for e in self.chain_to_root() {
            if let EntryPayload::Message(MessageRecord {
                meta: Some(MessageMeta { usage: Some(u), .. }),
                ..
            }) = &e.payload
            {
                total.input_tokens += u.input_tokens;
                total.output_tokens += u.output_tokens;
            }
        }
        total
    }

    /// Proactive retrieval: find the most relevant older entries from the
    /// projected history, returning their entry IDs and messages.
    ///
    /// This is a projection-only operation — it returns entries that are
    /// already in the projection. No ledger mutation, no new entries.
    ///
    /// Scoring uses the same BM25+entity-bonus approach as cross-session
    /// search, but scored against the current query over the in-memory
    /// projection. The result is capped at `cap` entries and excludes the
    /// `exclude_tail` verbatim tail (e.g. `keep_recent`).
    pub fn retrieve_relevant_entries(
        &self,
        query: &str,
        cap: usize,
        exclude_tail: usize,
    ) -> Vec<(String, vak_llm::Message)> {
        if query.trim().is_empty() || cap == 0 {
            return Vec::new();
        }
        let tagged = self.derive_keyed_tagged();
        if tagged.is_empty() {
            return Vec::new();
        }

        let terms: Vec<String> = crate::search::tokenize_impl(query);
        let phrase = crate::search::normalize_impl(query);
        if terms.is_empty() || phrase.is_empty() {
            return Vec::new();
        }

        let candidates: Vec<_> = if tagged.len() > exclude_tail {
            tagged[..tagged.len() - exclude_tail]
                .iter()
                .filter(|(_, _, is_summary, is_control)| !*is_summary && !*is_control)
                .collect()
        } else {
            Vec::new()
        };

        if candidates.is_empty() {
            return Vec::new();
        }

        let mut scored: Vec<(f32, &String, &vak_llm::Message)> = Vec::new();
        for (id, msg, _, _) in &candidates {
            let text = msg.text_content();
            let entities = crate::search::extract_entities(&text);
            let normalized = crate::search::normalize_impl(&text);
            let score = crate::search::score_normalized(&normalized, &terms, &phrase, &entities);
            if score > 0.0 {
                scored.push((score, id, msg));
            }
        }

        // Sort by score descending, take the top `cap`.
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored
            .into_iter()
            .take(cap)
            .map(|(_, id, m)| (id.clone(), m.clone()))
            .collect()
    }
}

pub struct SessionPath;

impl SessionPath {
    pub fn sessions_dir(home: &Path, cwd: &Path) -> PathBuf {
        home.join("sessions").join(hash_cwd(cwd))
    }

    pub fn new_session_file(home: &Path, cwd: &Path, session_id: &str) -> PathBuf {
        Self::sessions_dir(home, cwd).join(format!("{session_id}.jsonl"))
    }
}

/// FNV-1a: a fixed hash whose output does not change across Rust releases,
/// unlike DefaultHasher (SipHash with randomly-seeded-but-toolchain-chosen
/// parameters). Session directories must remain reachable after toolchain
/// upgrades.
fn hash_cwd(cwd: &Path) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in cwd.to_string_lossy().as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}
