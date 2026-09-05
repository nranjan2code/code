use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use vak_llm::Message;

use crate::types::{
    CompactionPlan, Entry, EntryPayload, MessageMeta, MessageRecord, SessionError, SessionHeader,
    WorkEvent,
};

pub struct SessionLog {
    path: PathBuf,
    file: File,
    entries: Vec<Entry>,
    by_id: HashMap<String, usize>,
    tail_id: Option<String>,
    tail_hash: Option<String>,
    warnings: Vec<String>,
}

impl SessionLog {
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
            entries: Vec::new(),
            by_id: HashMap::new(),
            tail_id: None,
            tail_hash: None,
            warnings: Vec::new(),
        };
        log.append(Entry::new(None, EntryPayload::Header(header)))?;
        Ok(log)
    }

    pub fn open(path: PathBuf) -> Result<Self, SessionError> {
        let file = OpenOptions::new().append(true).open(&path)?;
        file.try_lock()
            .map_err(|_| SessionError::Locked(path.clone()))?;
        let reader = BufReader::new(File::open(&path)?);
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
        Ok(SessionLog {
            path,
            file,
            entries,
            by_id,
            tail_id,
            tail_hash,
            warnings,
        })
    }

    /// Non-fatal problems seen while opening the ledger.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn append(&mut self, entry: Entry) -> Result<Entry, SessionError> {
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

    /// Appends a presentation/audit lifecycle fact. It is deliberately
    /// excluded from `derive_messages`.
    pub fn append_activity(
        &mut self,
        activity: crate::types::ActivityRecord,
    ) -> Result<Entry, SessionError> {
        let parent = self.tail_id.clone();
        self.append(Entry::new(parent, EntryPayload::Activity(activity)))
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
        let Some(projection) = self
            .work_projection()
            .map_err(|error| SessionError::Corrupt {
                line: 0,
                message: error.to_string(),
            })?
        else {
            return Ok(0);
        };
        let stale: Vec<(String, u32)> = projection
            .items
            .values()
            .filter(|item| {
                item.status == crate::types::WorkItemStatus::Running
                    && !item
                        .child_session_id
                        .as_ref()
                        .is_some_and(|id| live_child_sessions.contains(id))
            })
            .map(|item| (item.item_id.clone(), item.attempt))
            .collect();
        let mut reconciled = 0;
        for (item_id, attempt) in stale {
            let Some(current) = self
                .work_projection()
                .map_err(|error| SessionError::Corrupt {
                    line: 0,
                    message: error.to_string(),
                })?
            else {
                break;
            };
            self.append_work(WorkEvent {
                contract_id: current.contract.contract_id.clone(),
                revision: current.contract.revision,
                kind: crate::types::WorkEventKind::ItemStatusChanged {
                    item_id,
                    from: crate::types::WorkItemStatus::Running,
                    to: crate::types::WorkItemStatus::Interrupted,
                    attempt,
                    reason: "recovered after process restart; review before retry".into(),
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

    /// Like `derive_keyed`, additionally flagging compaction-summary
    /// pseudo-entries so packet accounting can exclude them from both
    /// sides of a partition (they were settled by earlier compactions).
    fn derive_keyed_tagged(&self) -> Vec<(String, Message, bool, bool)> {
        let mut out: Vec<(String, Message, bool, bool)> = Vec::new();
        // Only the newest intent note applies. It is emitted in its own
        // position in the chain — immediately before the turn it belongs to —
        // rather than prepended, because per-turn operating guidance stranded
        // at the top of a long conversation is guidance the model has stopped
        // paying attention to by the time it matters.
        let latest_intent = self
            .chain_to_root()
            .iter()
            .rev()
            .find(|entry| {
                matches!(&entry.payload, EntryPayload::Intent(record) if record.model_visible.is_some())
            })
            .map(|entry| entry.id.clone());
        for entry in self.chain_to_root() {
            match &entry.payload {
                EntryPayload::Message(record) => {
                    out.push((entry.id.clone(), record.message.clone(), false, false));
                }
                EntryPayload::Compaction(c) => {
                    if c.reset_all {
                        // Reset-with-handoff: everything becomes the summary.
                        out.drain(..);
                    } else {
                        let keep_from = out
                            .iter()
                            .position(|(id, _, _, _)| id == &c.first_kept_entry_id)
                            .unwrap_or(out.len());
                        out.drain(..keep_from);
                    }
                    let summary_msg = Message::user_text(format!(
                        "<context_summary>\n{}\n</context_summary>",
                        c.summary
                    ));
                    out.insert(0, (entry.id.clone(), summary_msg, true, false));
                }
                EntryPayload::Intent(record) => {
                    // Superseded intent notes contribute nothing: replaying
                    // five of them wastes context and lets a stale instruction
                    // argue with the current one.
                    if latest_intent.as_deref() != Some(entry.id.as_str()) {
                        continue;
                    }
                    if let Some(note) = &record.model_visible {
                        let mut block = format!("<intent>\n{note}\n</intent>");
                        block.truncate(4_000);
                        // Tagged as control: it is runtime-generated guidance
                        // for the turn in flight, not conversation to be
                        // summarized into a compaction packet.
                        out.push((entry.id.clone(), Message::user_text(block), false, true));
                    }
                }
                // Receipts and goal entries are audit, not model-visible input.
                EntryPayload::Header(_)
                | EntryPayload::Receipt(_)
                | EntryPayload::Goal(_)
                | EntryPayload::Activity(_)
                | EntryPayload::Work(_)
                | EntryPayload::TurnCapabilitiesBound(_) => {}
            }
        }
        if let Ok(Some(work)) = self.work_projection()
            && !matches!(
                work.status,
                crate::types::WorkContractStatus::Completed
                    | crate::types::WorkContractStatus::Failed
                    | crate::types::WorkContractStatus::Cancelled
                    | crate::types::WorkContractStatus::Unverified
            )
        {
            let mut context = format!(
                "<work_contract id=\"{}\" revision=\"{}\">\nObjective: {}\nStatus: {:?}\nItems:\n",
                work.contract.contract_id,
                work.contract.revision,
                work.contract.objective,
                work.status,
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
            let work_entry_id = self
                .chain_to_root()
                .iter()
                .rev()
                .find(|entry| matches!(entry.payload, EntryPayload::Work(_)))
                .map(|entry| entry.id.clone())
                .unwrap_or_else(|| work.contract.contract_id.clone());
            out.insert(0, (work_entry_id, Message::user_text(context), false, true));
        }

        out
    }

    /// Packet accounting for a planned boundary: message entries before
    /// `first_kept_entry_id` in the current projection become `dropped`,
    /// the rest stay `selected`.
    fn partition_at_boundary(
        tagged: &[(String, Message, bool, bool)],
        boundary: usize,
    ) -> crate::types::ContextPartition {
        let mut selected = Vec::new();
        let mut dropped = Vec::new();
        for (i, (id, _, is_summary, is_control)) in tagged.iter().enumerate() {
            if *is_summary || *is_control {
                continue;
            }
            if i < boundary {
                dropped.push(id.clone());
            } else {
                selected.push(id.clone());
            }
        }
        crate::types::ContextPartition {
            selected_entry_ids: selected,
            dropped_entry_ids: dropped,
        }
    }

    pub fn derive_messages(&self) -> Vec<Message> {
        self.derive_keyed().into_iter().map(|(_, m)| m).collect()
    }

    /// A projection-based compaction plan: `older` is everything before the
    /// snapped boundary (prior `<context_summary>` entries included, so
    /// repeated compaction summarizes summaries, not raw history), `keep`
    /// is the verbatim tail. The boundary never splits an assistant
    /// tool_use / user tool_result pair — it advances past result-bearing
    /// user messages so the kept region always starts API-valid.
    pub fn plan_compaction(&self, keep_recent: usize) -> Option<CompactionPlan> {
        let tagged = self.derive_keyed_tagged();
        if tagged.len() <= keep_recent {
            return None;
        }
        let mut boundary = tagged.len() - keep_recent;
        let has_tool_result = |m: &Message| {
            m.content
                .iter()
                .any(|b| matches!(b, vak_llm::ContentBlock::ToolResult { .. }))
        };
        while boundary < tagged.len() && has_tool_result(&tagged[boundary].1) {
            boundary += 1;
        }
        if boundary == 0 {
            return None;
        }
        Some(CompactionPlan {
            older: tagged[..boundary]
                .iter()
                .filter(|(_, _, _, is_control)| !*is_control)
                .map(|(_, m, _, _)| m.clone())
                .collect(),
            // Anchor = FIRST KEPT entry: the walker drains everything
            // strictly before this id and inserts the summary at index 0.
            first_kept_entry_id: tagged[boundary].0.clone(),
            partition: Self::partition_at_boundary(&tagged, boundary),
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
