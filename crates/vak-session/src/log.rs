use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use vak_llm::Message;

use crate::types::{
    CompactionPlan, Entry, EntryPayload, MessageMeta, MessageRecord, SessionError, SessionHeader,
};

pub struct SessionLog {
    path: PathBuf,
    file: File,
    entries: Vec<Entry>,
    by_id: HashMap<String, usize>,
    tail_id: Option<String>,
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
                continue;
            };
            by_id.insert(entry.id.clone(), entries.len());
            entries.push(entry);
        }
        let tail_id = entries.last().map(|e| e.id.clone());
        Ok(SessionLog {
            path,
            file,
            entries,
            by_id,
            tail_id,
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
        let line = serde_json::to_string(&entry).map_err(|e| SessionError::Corrupt {
            line: 0,
            message: e.to_string(),
        })?;
        writeln!(self.file, "{line}")?;
        self.file.flush()?;
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

    /// Distinct bash commands that ran GREEN on the active chain, in
    /// first-run order (doc 27 Phase E adoption substrate). A command is
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

    /// Reset-with-handoff (doc 27 Phase H): the projection becomes ONLY
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
            .map(|(id, m, _)| (id, m))
            .collect()
    }

    /// Like `derive_keyed`, additionally flagging compaction-summary
    /// pseudo-entries so packet accounting can exclude them from both
    /// sides of a partition (they were settled by earlier compactions).
    fn derive_keyed_tagged(&self) -> Vec<(String, Message, bool)> {
        let mut out: Vec<(String, Message, bool)> = Vec::new();
        for entry in self.chain_to_root() {
            match &entry.payload {
                EntryPayload::Message(record) => {
                    out.push((entry.id.clone(), record.message.clone(), false));
                }
                EntryPayload::Compaction(c) => {
                    if c.reset_all {
                        // Reset-with-handoff: everything becomes the summary.
                        out.drain(..);
                    } else {
                        let keep_from = out
                            .iter()
                            .position(|(id, _, _)| id == &c.first_kept_entry_id)
                            .unwrap_or(out.len());
                        out.drain(..keep_from);
                    }
                    let summary_msg = Message::user_text(format!(
                        "<context_summary>\n{}\n</context_summary>",
                        c.summary
                    ));
                    out.insert(0, (entry.id.clone(), summary_msg, true));
                }
                // Receipts and goal entries are audit, not model-visible input.
                EntryPayload::Header(_) | EntryPayload::Receipt(_) | EntryPayload::Goal(_) => {}
            }
        }
        out
    }

    /// Packet accounting for a planned boundary: message entries before
    /// `first_kept_entry_id` in the current projection become `dropped`,
    /// the rest stay `selected`.
    fn partition_at_boundary(
        tagged: &[(String, Message, bool)],
        boundary: usize,
    ) -> crate::types::ContextPartition {
        let mut selected = Vec::new();
        let mut dropped = Vec::new();
        for (i, (id, _, is_summary)) in tagged.iter().enumerate() {
            if *is_summary {
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
                .map(|(_, m, _)| m.clone())
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
