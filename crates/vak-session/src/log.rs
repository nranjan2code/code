use std::collections::HashMap;
use std::path::{Path, PathBuf};

use vak_llm::Message;

use crate::turns::{Evidence, Fidelity, Packet, TurnCard, TurnIndex, WorkingSetPlan};
use crate::types::{
    Entry, EntryPayload, MessageMeta, MessageRecord, PresentationRecord, SessionError,
    SessionHeader, TranscriptMessage, TurnCardRecord, WorkEvent,
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
    /// What this session has changed in the workspace so far, for a turn
    /// whose context profile is `working` or `full`. Read from the
    /// `workspace_delta` activity the host recorded for this turn, so the
    /// bytes the model saw are the bytes in the ledger.
    pub workspace: Option<String>,
}

/// The ledger's descriptor. A writable handle holds the ledger's exclusive
/// lock and releases it with an explicit unlock when dropped. Closing alone
/// would not: a lock lasts while any duplicate of its descriptor is open,
/// and a child being spawned holds duplicates of every open descriptor
/// until it execs, so a ledger dropped and reopened while any thread was
/// spawning a process failed with `Locked` although nothing held it.
/// A session ledger on disk: a directory of record segments
/// (`vak_storage::segments`, docs/design/73 §6). A writer holds `LOCK`
/// exclusively for the handle's lifetime, so two processes never interleave
/// appends; a read-only handle holds no lock and no writer.
struct LedgerDir {
    segments: vak_storage::segments::SegmentSet,
    writer: Option<(u64, vak_storage::records::RecordWriter)>,
    /// The segment set's single-writer lock, held for the handle's life.
    lock: Option<vak_storage::segments::WriterLock>,
}

impl LedgerDir {
    /// The ledger's segments and its writer lock, which no other handle
    /// (in this or another process) holds while this one lives.
    fn locked(
        dir: &Path,
    ) -> Result<
        (
            vak_storage::segments::SegmentSet,
            vak_storage::segments::WriterLock,
        ),
        SessionError,
    > {
        let segments = vak_storage::segments::SegmentSet::open(dir).map_err(storage_error)?;
        let lock = segments
            .try_lock()
            .map_err(storage_error)?
            .ok_or_else(|| SessionError::Locked(dir.to_path_buf()))?;
        Ok((segments, lock))
    }
}

/// Whether `entry` is a commit point, synced with everything before it:
/// the header, what a person or a tool result put in (the directive, a
/// steering message, tool results; never a runtime nudge), an effect outside the ledger, and the
/// card that closes a turn. Everything else becomes durable at the next
/// commit point or when the ledger closes.
fn commits(entry: &Entry) -> bool {
    match &entry.payload {
        EntryPayload::Header(_) | EntryPayload::TurnCard(_) | EntryPayload::CallEffect(_) => true,
        EntryPayload::Message(record) => {
            record.message.role == vak_llm::Role::User && record.control_kind().is_none()
        }
        _ => false,
    }
}

/// The segment numbers present in a ledger directory, ascending, and
/// whether the newest is still open (has a `.log`).
pub(crate) fn segment_numbers(dir: &Path) -> Vec<(u64, bool)> {
    let mut found: std::collections::BTreeMap<u64, bool> = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(rest) = name.strip_prefix("seg-") else {
            continue;
        };
        let (number, open) = if let Some(n) = rest.strip_suffix(".log") {
            (n, true)
        } else if let Some(n) = rest.strip_suffix(".sealed") {
            (n, false)
        } else {
            continue;
        };
        if let Ok(number) = number.parse::<u64>() {
            let slot = found.entry(number).or_insert(false);
            *slot |= open;
        }
    }
    found.into_iter().collect()
}

fn storage_error(error: vak_storage::StorageError) -> SessionError {
    match error {
        vak_storage::StorageError::Io(io) => SessionError::Io(io),
        other => SessionError::Corrupt {
            line: 0,
            message: other.to_string(),
        },
    }
}

pub struct SessionLog {
    path: PathBuf,
    ledger: LedgerDir,
    entries: Vec<Entry>,
    by_id: HashMap<String, usize>,
    tail_id: Option<String>,
    warnings: Vec<String>,
    /// The turn later appends belong to: the latest directive on the active
    /// chain, or the id reserved by `begin_turn` for the next one.
    current_turn: Option<String>,
    /// An id `begin_turn` reserved; the next directive appended takes it.
    reserved_turn: Option<String>,
    /// Where this ledger's large payloads live; attached by the owner of
    /// the tenant (`with_objects`). Writing one without it fails closed.
    objects: Option<std::sync::Arc<dyn crate::objects::Objects>>,
    /// Bodies already read through `objects`, by object id.
    fetched: std::sync::Mutex<HashMap<String, String>>,
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
        vak_config::spaces::require_bound(&path).map_err(SessionError::Unbound)?;
        crate::fence::check()?;
        std::fs::create_dir_all(&path)?;
        // Cross-process safety: an exclusive lock for the lifetime of the
        // handle keeps two processes from interleaving appends.
        let (segments, lock) = LedgerDir::locked(&path)?;
        if !segment_numbers(&path).is_empty() {
            return Err(SessionError::Exists(path));
        }
        let writer = segments.writer(1, &lock).map_err(storage_error)?;
        let mut log = SessionLog {
            path,
            ledger: LedgerDir {
                segments,
                writer: Some((1, writer)),
                lock: Some(lock),
            },
            entries: Vec::new(),
            by_id: HashMap::new(),
            tail_id: None,
            warnings: Vec::new(),
            current_turn: None,
            reserved_turn: None,
            objects: None,
            fetched: Default::default(),
        };
        log.append(Entry::new(None, EntryPayload::Header(header)))?;
        Ok(log)
    }
}

struct ParsedEntries {
    entries: Vec<Entry>,
    by_id: HashMap<String, usize>,
    tail_id: Option<String>,
    warnings: Vec<String>,
}

impl SessionLog {
    /// Reads every segment of the ledger at `path`. A segment whose chain
    /// fails to verify is read frame by frame anyway and reported: the
    /// record is evidence, and refusing to open it would hide the only copy.
    /// A torn final frame (crash mid-append) ends that segment's entries.
    fn parse_entries(path: &Path) -> Result<ParsedEntries, SessionError> {
        if !path.is_dir() {
            return Err(SessionError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no session ledger at {}", path.display()),
            )));
        }
        let segments = vak_storage::segments::SegmentSet::open(path).map_err(storage_error)?;
        let mut entries = Vec::new();
        let mut by_id = HashMap::new();
        let mut warnings = Vec::new();
        for (number, _) in segment_numbers(path) {
            let raw = match segments.read(number, None) {
                Ok(raw) => raw,
                Err(error) => {
                    warnings.push(format!(
                        "ledger {} segment {number} failed verification ({error}); \
                         its entries were read without it",
                        path.display()
                    ));
                    let file = if segments.log_path(number).exists() {
                        segments.log_path(number)
                    } else {
                        segments.sealed_path(number)
                    };
                    vak_storage::segments::frame_bytes(&file)
                        .and_then(|bytes| vak_storage::records::located_entries(&bytes, 0))
                        .map(|located| located.into_iter().map(|e| e.entry).collect())
                        .unwrap_or_default()
                }
            };
            for (i, bytes) in raw.iter().enumerate() {
                let Ok(entry) = serde_json::from_slice::<Entry>(bytes) else {
                    warnings.push(format!(
                        "skipped unparseable entry {} of segment {number} in {}",
                        i + 1,
                        path.display()
                    ));
                    continue;
                };
                by_id.insert(entry.id.clone(), entries.len());
                entries.push(entry);
            }
        }
        let tail_id = entries.last().map(|e| e.id.clone());
        Ok(ParsedEntries {
            entries,
            by_id,
            tail_id,
            warnings,
        })
    }

    /// Read one located canonical record without materializing its prefix.
    /// This low-level primitive grants no access: Core validates the requested
    /// session, workspace, agent/audience and trash before calling it.
    pub fn read_record_at(
        path: &Path,
        offset: u64,
        length: u64,
        expected_id: &str,
        expected_digest: &str,
    ) -> Result<Entry, SessionError> {
        if length == 0 || length > 64 * 1024 * 1024 {
            return Err(SessionError::Corrupt {
                line: 0,
                message: "invalid indexed record size".into(),
            });
        }
        let bytes = vak_storage::segments::frame_bytes(path).map_err(storage_error)?;
        let entry_bytes = vak_storage::records::entry_at(&bytes, offset, length).map_err(|_| {
            SessionError::Corrupt {
                line: 0,
                message: "indexed record is outside the ledger".into(),
            }
        })?;
        let line = std::str::from_utf8(&entry_bytes).map_err(|error| SessionError::Corrupt {
            line: 0,
            message: error.to_string(),
        })?;
        if crate::types::line_digest(line) != expected_digest {
            return Err(SessionError::Corrupt {
                line: 0,
                message: "indexed record changed; rebuild the index".into(),
            });
        }
        let entry: Entry = serde_json::from_str(line).map_err(|error| SessionError::Corrupt {
            line: 0,
            message: error.to_string(),
        })?;
        if entry.id != expected_id {
            return Err(SessionError::Corrupt {
                line: 0,
                message: "indexed entry identity mismatch".into(),
            });
        }
        Ok(entry)
    }

    pub fn open(path: PathBuf) -> Result<Self, SessionError> {
        crate::fence::check()?;
        // Opening never creates: a ledger that is not there is not found.
        if !path.is_dir() {
            return Err(SessionError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no ledger at {}", path.display()),
            )));
        }
        let (segments, lock) = LedgerDir::locked(&path)?;
        let numbers = segment_numbers(&path);
        // Append to the newest segment while it is open; after a seal, the
        // next one.
        let active = match numbers.last() {
            Some((number, true)) => *number,
            Some((number, false)) => number + 1,
            None => 1,
        };
        let log_path = segments.log_path(active);
        let mut repaired = None;
        if log_path.exists() {
            let prev = segments.prev_head(active).map_err(storage_error)?;
            let report =
                vak_storage::records::verify_chain_from(&log_path, prev).map_err(storage_error)?;
            if let Some(at) = report.torn_tail {
                vak_storage::records::truncate_torn_tail_from(&log_path, prev)
                    .map_err(storage_error)?;
                repaired = Some(at);
            }
        }
        let mut parsed = Self::parse_entries(&path)?;
        if let Some(at) = repaired {
            parsed.warnings.push(format!(
                "ledger {} segment {active} ended in a torn frame at byte {at} \
                 (a crash mid-append); it was cut off before this write",
                path.display()
            ));
        }
        let writer = segments.writer(active, &lock).map_err(storage_error)?;
        Ok(SessionLog {
            path,
            ledger: LedgerDir {
                segments,
                writer: Some((active, writer)),
                lock: Some(lock),
            },
            entries: parsed.entries,
            by_id: parsed.by_id,
            tail_id: parsed.tail_id,
            warnings: parsed.warnings,
            current_turn: None,
            reserved_turn: None,
            objects: None,
            fetched: Default::default(),
        }
        .with_current_turn())
    }

    /// Open an existing session for reading and inspection without acquiring an
    /// exclusive write lock. Allows web clients, exports, and inspectors to
    /// read and rehydrate sessions that are currently active in another process.
    pub fn open_read_only(path: PathBuf) -> Result<Self, SessionError> {
        let parsed = Self::parse_entries(&path)?;
        let segments = vak_storage::segments::SegmentSet::open(&path).map_err(storage_error)?;
        Ok(SessionLog {
            path,
            ledger: LedgerDir {
                segments,
                writer: None,
                lock: None,
            },
            entries: parsed.entries,
            by_id: parsed.by_id,
            tail_id: parsed.tail_id,
            warnings: parsed.warnings,
            current_turn: None,
            reserved_turn: None,
            objects: None,
            fetched: Default::default(),
        }
        .with_current_turn())
    }

    /// Seals the open segment (copy, verify, atomic swap, seal entry) and
    /// starts the next, so a long session's history compacts without any
    /// entry being rewritten (docs/design/73 §6, the invariant 2 amendment).
    pub fn seal_segment(&mut self) -> Result<(), SessionError> {
        let Some((number, writer)) = self.ledger.writer.take() else {
            return Err(SessionError::Locked(self.path.clone()));
        };
        let mut writer = writer;
        writer.sync().map_err(storage_error)?;
        drop(writer);
        let Some(lock) = self.ledger.lock.as_ref() else {
            return Err(SessionError::Locked(self.path.clone()));
        };
        self.ledger
            .segments
            .seal(number, lock)
            .map_err(storage_error)?;
        let next = self
            .ledger
            .segments
            .writer(number + 1, lock)
            .map_err(storage_error)?;
        self.ledger.writer = Some((number + 1, next));
        Ok(())
    }

    /// The header of the ledger at `path`, read from its first frame alone,
    /// so listing many sessions never parses whole transcripts.
    pub fn read_header(path: &Path) -> Result<SessionHeader, SessionError> {
        let first = SessionLog::segment_files(path)
            .into_iter()
            .next()
            .ok_or_else(|| SessionError::Corrupt {
                line: 0,
                message: "empty session ledger".into(),
            })?;
        let bytes = vak_storage::segments::frame_bytes(&first).map_err(storage_error)?;
        let located = vak_storage::records::located_entries(&bytes, 0).map_err(storage_error)?;
        let entry = located.first().ok_or_else(|| SessionError::Corrupt {
            line: 0,
            message: "empty session ledger".into(),
        })?;
        match serde_json::from_slice::<Entry>(&entry.entry) {
            Ok(Entry {
                payload: EntryPayload::Header(header),
                ..
            }) => Ok(header),
            Ok(_) => Err(SessionError::Corrupt {
                line: 1,
                message: "session ledger has no header".into(),
            }),
            Err(error) => Err(SessionError::Corrupt {
                line: 1,
                message: error.to_string(),
            }),
        }
    }

    /// Visits the ledger's entries in order without verifying the chain or
    /// building the in-memory model, stopping when `visit` returns `false`.
    /// For listings that summarise many sessions; returns the number
    /// visited. An entry that does not parse is counted and skipped.
    pub fn scan(path: &Path, mut visit: impl FnMut(Option<&Entry>) -> bool) -> u64 {
        let mut count = 0;
        for segment in SessionLog::segment_files(path) {
            let Ok(bytes) = vak_storage::segments::frame_bytes(&segment) else {
                continue;
            };
            let Ok(located) = vak_storage::records::located_entries(&bytes, 0) else {
                continue;
            };
            for frame in located {
                count += 1;
                let entry = serde_json::from_slice::<Entry>(&frame.entry).ok();
                if !visit(entry.as_ref()) {
                    return count;
                }
            }
        }
        count
    }

    /// The ledger's entries as JSON, one per line, in order: a plain-text
    /// rendering for inspection and tests. Not a format anything parses back.
    pub fn text(path: &Path) -> String {
        let mut out = String::new();
        for segment in SessionLog::segment_files(path) {
            let Ok(bytes) = vak_storage::segments::frame_bytes(&segment) else {
                continue;
            };
            for frame in vak_storage::records::located_entries(&bytes, 0).unwrap_or_default() {
                out.push_str(&String::from_utf8_lossy(&frame.entry));
                out.push('\n');
            }
        }
        out
    }

    /// When the ledger last changed: its newest segment's modification time.
    pub fn modified(path: &Path) -> Option<std::time::SystemTime> {
        SessionLog::segment_files(path)
            .iter()
            .filter_map(|segment| std::fs::metadata(segment).ok()?.modified().ok())
            .max()
    }

    /// The segment files of this ledger, oldest first: what an index reads.
    pub fn segment_files(path: &Path) -> Vec<PathBuf> {
        let Ok(segments) = vak_storage::segments::SegmentSet::open(path) else {
            return Vec::new();
        };
        segment_numbers(path)
            .into_iter()
            .map(|(number, open)| {
                if open {
                    segments.log_path(number)
                } else {
                    segments.sealed_path(number)
                }
            })
            .collect()
    }

    /// Returns whether this SessionLog was opened read-only.
    pub fn is_read_only(&self) -> bool {
        self.ledger.writer.is_none()
    }

    /// Non-fatal problems seen while opening the ledger.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn append(&mut self, entry: Entry) -> Result<Entry, SessionError> {
        if self.ledger.writer.is_none() {
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
        if entry.is_directive() {
            if let Some(reserved) = self.reserved_turn.take() {
                if self.by_id.contains_key(&reserved) {
                    return Err(SessionError::Corrupt {
                        line: 0,
                        message: format!("turn id {reserved} is already an entry"),
                    });
                }
                entry.id = reserved;
            }
            self.current_turn = Some(entry.id.clone());
        }
        if entry.at_turn.is_none() {
            entry.at_turn = self.current_turn.clone();
        }
        let line = serde_json::to_string(&entry).map_err(|e| SessionError::Corrupt {
            line: 0,
            message: e.to_string(),
        })?;
        let Some((_, writer)) = self.ledger.writer.as_mut() else {
            return Err(SessionError::Locked(self.path.clone()));
        };
        // One frame per entry; the frame chain over the stored bytes is the
        // ledger's integrity (docs/design/73 §6). Group commit: a commit
        // point syncs every frame before it, so a turn costs a few syncs,
        // not one per entry.
        writer
            .append_unsynced(line.as_bytes(), None)
            .map_err(storage_error)?;
        if commits(&entry) {
            writer.sync().map_err(storage_error)?;
        }
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

    /// Appends one effect of a tool call (audit; never model-visible).
    pub fn append_call_effect(
        &mut self,
        record: crate::types::CallEffectRecord,
    ) -> Result<Entry, SessionError> {
        let parent = self.tail_id.clone();
        self.append(Entry::new(parent, EntryPayload::CallEffect(record)))
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

    /// The latest goal update on the active branch. An update on a branch
    /// the conversation left behind is not part of its goal.
    pub fn latest_goal_update(&self) -> Option<vak_intent::GoalUpdate> {
        self.active_entries_rev()
            .find_map(|entry| match &entry.payload {
                EntryPayload::GoalUpdate(update) => Some(update.clone()),
                _ => None,
            })
    }

    /// How `text` relates to this conversation's goal, as the next update to
    /// append (`vak_intent::next_goal_update`).
    pub fn next_goal_update(&self, text: &str) -> vak_intent::GoalUpdate {
        vak_intent::next_goal_update(
            text,
            self.goal_state().as_ref(),
            self.latest_goal_update().map(|update| update.revision),
        )
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

    /// Appends an entry about an earlier turn (an outcome review, a file a
    /// Review promoted), named as that turn's rather than the current one's.
    /// `None` stamps it like any other entry.
    pub fn append_in_turn(
        &mut self,
        payload: EntryPayload,
        turn: Option<String>,
    ) -> Result<Entry, SessionError> {
        let parent = self.tail_id.clone();
        let mut entry = Entry::new(parent, payload);
        entry.at_turn = turn;
        self.append(entry)
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

    /// Attaches the tenant object store this ledger's large payloads use.
    pub fn with_objects(mut self, objects: std::sync::Arc<dyn crate::objects::Objects>) -> Self {
        self.set_objects(objects);
        self
    }

    pub fn set_objects(&mut self, objects: std::sync::Arc<dyn crate::objects::Objects>) {
        self.objects = Some(objects);
    }

    fn object_scope(&self) -> Result<String, SessionError> {
        self.header()
            .map(|header| crate::objects::conversation_scope(&header.session_id))
            .ok_or_else(|| SessionError::Objects("ledger has no header".into()))
    }

    /// Stores `bytes` as one of this conversation's objects.
    pub fn put_object(&self, bytes: &[u8]) -> Result<crate::objects::ObjectRef, SessionError> {
        let objects = self
            .objects
            .as_ref()
            .ok_or_else(|| SessionError::Objects("no object store attached".into()))?;
        objects.put(bytes, &self.object_scope()?)
    }

    /// Reads one of this conversation's objects as text, or `None` when no
    /// store is attached or the object cannot be read.
    pub fn object_text(&self, object: &crate::objects::ObjectRef) -> Option<String> {
        let mut fetched = self.fetched.lock().ok()?;
        if let Some(text) = fetched.get(&object.id) {
            return Some(text.clone());
        }
        let bytes = self
            .objects
            .as_ref()?
            .get(object, &self.object_scope().ok()?)
            .ok()?;
        let text = String::from_utf8(bytes).ok()?;
        fetched.insert(object.id.clone(), text.clone());
        Some(text)
    }

    /// Records the whole result behind a windowed `ToolResult` block
    /// (docs/design/68-context-engine.md §3) as an object, so `recall` and
    /// the closed-turn digests read what the tool returned, not what the
    /// request carried.
    pub fn append_evidence_body(
        &mut self,
        tool_use_id: &str,
        content: String,
    ) -> Result<Entry, SessionError> {
        let body = self.put_object(content.as_bytes())?;
        if let Ok(mut fetched) = self.fetched.lock() {
            fetched.insert(body.id.clone(), content);
        }
        let parent = self.tail_id.clone();
        self.append(Entry::new(
            parent,
            EntryPayload::EvidenceBody(crate::types::EvidenceBodyRecord {
                tool_use_id: tool_use_id.to_string(),
                body,
            }),
        ))
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
    /// call arguments, its whole result content (the evidence body when the
    /// request carried a window of it), and whether it failed — by
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
        let body = chain.iter().find_map(|entry| match &entry.payload {
            EntryPayload::EvidenceBody(body) if body.tool_use_id == tool_use_id => {
                self.object_text(&body.body)
            }
            _ => None,
        });
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
                        content: body.unwrap_or_else(|| content.clone()),
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

    /// The presentations recorded on the active, latest user turn only, so a
    /// card from an earlier turn cannot stand for this turn's evidence.
    pub fn presentations_for_latest_turn(&self) -> Vec<&PresentationRecord> {
        let mut found = Vec::new();
        for entry in self.chain_to_root() {
            match &entry.payload {
                EntryPayload::Message(record)
                    if record.message.role == vak_llm::Role::User
                        && record.control_kind().is_none()
                        && record
                            .message
                            .content
                            .iter()
                            .any(|block| matches!(block, vak_llm::ContentBlock::Text { .. })) =>
                {
                    found.clear();
                }
                EntryPayload::Presentation(record) => found.push(record),
                _ => {}
            }
        }
        found
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
        self.active_entries_rev()
            .find(|entry| entry.is_directive())
            .map(|entry| entry.id.clone())
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

    /// Records the interface a turn was bound to: in full when it differs
    /// from the last one this conversation bound, otherwise as a
    /// `TurnCapabilitiesRef` to that entry.
    pub fn append_turn_capabilities(
        &mut self,
        binding: crate::types::TurnBinding,
    ) -> Result<Entry, SessionError> {
        let interface = serde_json::to_vec(&serde_json::json!({
            "system_prompt": binding.system_prompt,
            "tool_schemas": binding.tool_schemas,
            "tool_index": binding.tool_index,
        }))
        .map_err(|error| SessionError::Objects(error.to_string()))?;
        let bound = crate::types::TurnCapabilitiesBound {
            epoch: binding.epoch,
            capability_ids: binding.capability_ids,
            excluded_ids: binding.excluded_ids,
            core_tool_names: binding.core_tool_names,
            deferred_tool_names: binding.deferred_tool_names,
            tool_domains: binding.tool_domains,
            interface: self.put_object(&interface)?,
        };
        let digest = bound.digest();
        let previous = self
            .active_entries_rev()
            .find_map(|entry| match &entry.payload {
                EntryPayload::TurnCapabilitiesBound(earlier) => {
                    Some((entry.id.clone(), earlier.digest()))
                }
                _ => None,
            });
        let parent = self.tail_id.clone();
        let payload = match previous {
            Some((entry, earlier)) if earlier == digest => {
                EntryPayload::TurnCapabilitiesRef(crate::types::TurnCapabilitiesRef {
                    entry,
                    digest,
                    epoch: bound.epoch,
                })
            }
            _ => EntryPayload::TurnCapabilitiesBound(bound),
        };
        self.append(Entry::new(parent, payload))
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
        self.reserved_turn = None;
        self.current_turn = self.latest_directive_entry_id();
        Ok(())
    }

    /// Opens a turn whose directive is not written yet. Everything appended
    /// from now on names `turn_id` as its turn, and the next directive is
    /// written with `turn_id` as its entry id, so the records admission
    /// writes first (the intent and its strands) and the turn the context
    /// engine indexes are one id (docs/design/85-turn-graph.md, G0).
    /// Refused once this process is fenced (`crate::fence`): a restored
    /// store takes no new turn from the writer it replaced.
    pub fn begin_turn(&mut self, turn_id: &str) -> Result<(), SessionError> {
        crate::fence::check()?;
        self.reserved_turn = Some(turn_id.to_string());
        self.current_turn = Some(turn_id.to_string());
        Ok(())
    }

    /// The turn later appends belong to.
    pub fn current_turn(&self) -> Option<&str> {
        self.current_turn.as_deref()
    }

    fn with_current_turn(mut self) -> Self {
        self.current_turn = self.latest_directive_entry_id();
        self
    }

    /// Appends one compaction packet over the inclusive turn range
    /// `first_turn_id..=last_turn_id` (docs/design/68-context-engine.md §4).
    /// Both ids must be directive entries on the chain. The packet is a
    /// cache keyed by that range: it never moves a boundary and never hides
    /// the turns it covers from a plan that wants them at `Full` or `Card`.
    pub fn append_packet(
        &mut self,
        first_turn_id: &str,
        last_turn_id: &str,
        model: &str,
        summary: String,
        tokens_before: u64,
    ) -> Result<Entry, SessionError> {
        for id in [first_turn_id, last_turn_id] {
            if !self.by_id.contains_key(id) {
                return Err(SessionError::Corrupt {
                    line: 0,
                    message: format!("unknown turn id {id} in packet range"),
                });
            }
        }
        let parent = self.tail_id.clone();
        self.append(Entry::new(
            parent,
            EntryPayload::Compaction(crate::types::CompactionEntry {
                summary,
                first_turn_id: first_turn_id.to_string(),
                last_turn_id: last_turn_id.to_string(),
                model: model.to_string(),
                tokens_before,
                reset_all: false,
                keeps_open_turn: false,
            }),
        ))
    }

    /// Reset-with-handoff (docs/design/42-managed-work-contracts.md): the projection becomes ONLY
    /// this summary. Append-only; the full history stays on disk. This is
    /// the one compaction entry that is a real boundary — the rescue for a
    /// profile with no usable horizon, where nothing is plannable.
    pub fn append_handoff_reset(
        &mut self,
        summary: String,
        tokens_before: u64,
        keeps_open_turn: bool,
    ) -> Result<Entry, SessionError> {
        let parent = self.tail_id.clone();
        self.append(Entry::new(
            parent,
            EntryPayload::Compaction(crate::types::CompactionEntry {
                summary,
                first_turn_id: String::new(),
                last_turn_id: String::new(),
                model: String::new(),
                tokens_before,
                reset_all: true,
                keeps_open_turn,
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

    /// Walk backwards without allocating a vector for the entire active chain.
    /// Consumers that need only the newest state can stop immediately.
    pub fn active_entries_rev(&self) -> impl Iterator<Item = &Entry> {
        let mut cursor = self.tail_id.as_deref();
        std::iter::from_fn(move || {
            let id = cursor?;
            let entry = self.entries.get(*self.by_id.get(id)?)?;
            cursor = entry.parent_id.as_deref();
            Some(entry)
        })
    }

    /// Inspect the latest message without constructing/cloning the whole chain.
    pub fn latest_message(&self) -> Option<&Message> {
        let mut cursor = self.tail_id.as_deref();
        while let Some(id) = cursor {
            let entry = self.entries.get(*self.by_id.get(id)?)?;
            if let EntryPayload::Message(record) = &entry.payload {
                return Some(&record.message);
            }
            cursor = entry.parent_id.as_deref();
        }
        None
    }

    /// Reconstruct the active turn only. Work is proportional to this turn,
    /// not to unrelated settled history. Include admission metadata preceding
    /// its directive, stopping before the previous message.
    pub fn current_turn_index(&self) -> TurnIndex {
        self.current_turn_index_at(self.tail_id.as_deref())
    }

    fn current_turn_index_at(&self, leaf: Option<&str>) -> TurnIndex {
        let mut entries = Vec::new();
        let mut cursor = leaf;
        let mut found_directive = false;
        while let Some(id) = cursor {
            let Some(entry) = self
                .by_id
                .get(id)
                .and_then(|position| self.entries.get(*position))
            else {
                break;
            };
            if found_directive && matches!(entry.payload, EntryPayload::Message(_)) {
                break;
            }
            if let EntryPayload::Message(record) = &entry.payload {
                found_directive = record.control_kind().is_none()
                    && record.message.role == vak_llm::Role::User
                    && record
                        .message
                        .content
                        .iter()
                        .any(|block| matches!(block, vak_llm::ContentBlock::Text { .. }))
                    && !record
                        .message
                        .content
                        .iter()
                        .any(|block| matches!(block, vak_llm::ContentBlock::ToolResult { .. }));
            }
            entries.push(entry);
            cursor = entry.parent_id.as_deref();
        }
        entries.reverse();
        TurnIndex::from_entries_resolving(entries, &|body| self.object_text(body))
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
    /// by `derive_messages` (goal audits, search, human-facing views), which
    /// has no `CapacityProfile` to plan against.
    fn derive_keyed_tagged(&self) -> Vec<(String, Message, bool, bool)> {
        self.derive_with_plan_tagged(None)
    }

    /// Like `derive_keyed_tagged`, but a `WorkingSetPlan` (from
    /// `vak_context::planner::plan`) selects each closed turn's fidelity
    /// instead of defaulting every one to `Full` (docs/design/68-context-
    /// engine.md §4/§10) — one implementation, the plan just picks what each
    /// turn contributes:
    ///
    /// - `Full` turns project their `full_record`: real `tool_use` blocks,
    ///   results as schema-driven digests carrying their evidence id, never
    ///   a character-count trim.
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
    /// The reset boundary: the chain position before which everything is
    /// invisible to the model, and the handoff summary that stands in for
    /// it. Only a `reset_all` compaction entry (reset-with-handoff,
    /// docs/design/42) moves this; packets never do. `(0, ..., None)` when
    /// no reset has happened.
    fn reset_boundary(&self) -> (usize, HashMap<String, usize>, Option<String>) {
        let chain = self.chain_to_root();
        let position: HashMap<String, usize> = chain
            .iter()
            .enumerate()
            .map(|(i, entry)| (entry.id.clone(), i))
            .collect();
        let last_reset =
            chain
                .iter()
                .enumerate()
                .rev()
                .find_map(|(pos, entry)| match &entry.payload {
                    EntryPayload::Compaction(c) if c.reset_all => Some((pos, c.summary.clone())),
                    _ => None,
                });
        match last_reset {
            Some((pos, summary)) => (pos, position, Some(summary)),
            None => (0, position, None),
        }
    }

    /// Every stored packet (non-reset compaction entry) in ledger order.
    fn packets(&self) -> Vec<Packet> {
        TurnIndex::from_log(self).packets
    }

    /// The stored packet whose range is exactly `first_turn_id..=last_turn_id`,
    /// newest such entry first (docs/design/68 §4: a packet is reused only
    /// for the exact range the plan asks for). `None` when no packet
    /// covers that range, whatever other packets exist.
    pub fn packet_for(&self, first_turn_id: &str, last_turn_id: &str) -> Option<Packet> {
        self.packets().into_iter().rev().find(|packet| {
            packet.first_turn_id == first_turn_id && packet.last_turn_id == last_turn_id
        })
    }

    /// Whether the packet range a `WorkingSetPlan` asked for
    /// (`packet_range = (first, last)`) still needs a summariser call: true
    /// when no stored packet covers exactly that range.
    pub fn packet_needs_compaction(&self, first_turn_id: &str, last_turn_id: &str) -> bool {
        self.packet_for(first_turn_id, last_turn_id).is_none()
    }

    /// The summariser input for a packet over `first_turn_id..=last_turn_id`:
    /// the longest stored packet that starts at the same turn and ends at or
    /// before `last_turn_id` (so work already folded in is reused, never
    /// re-read from raw history), followed by one `TurnCard` line per turn
    /// after it up to `last_turn_id` — cards, never raw history (§4). Also
    /// returns a char-count estimate for the caller's `tokens_before`.
    pub fn packet_transcript(&self, first_turn_id: &str, last_turn_id: &str) -> (String, u64) {
        let (_, position, _) = self.reset_boundary();
        let mut out = String::new();
        let (Some(&first_pos), Some(&last_pos)) =
            (position.get(first_turn_id), position.get(last_turn_id))
        else {
            return (out, 0);
        };
        // Seed: the stored packet with this `first` and the greatest `last`
        // not past our target.
        let seed = self
            .packets()
            .into_iter()
            .filter(|packet| packet.first_turn_id == first_turn_id)
            .filter_map(|packet| {
                let end = position.get(packet.last_turn_id.as_str()).copied()?;
                (end <= last_pos).then_some((end, packet))
            })
            .max_by_key(|(end, _)| *end);
        let mut from_pos = first_pos;
        if let Some((end, packet)) = seed {
            out.push_str(&packet.summary);
            out.push_str("\n\n");
            from_pos = end + 1;
        }
        let index = TurnIndex::from_log(self);
        for (turn_number, turn) in index.turns.iter().enumerate() {
            let turn_pos = position.get(turn.id.as_str()).copied().unwrap_or(0);
            if turn_pos < from_pos || turn_pos > last_pos {
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

    /// The model-visible projection for one request. With a plan, each
    /// closed turn contributes exactly what the plan chose for it; without
    /// one, every closed turn rides at `Full` (the plan-free view used by
    /// goal audits and search, which have no `CapacityProfile`). Only the
    /// reset boundary (reset-with-handoff) hides anything; a stored packet
    /// is rendered only when the plan's `packet_range` matches it exactly,
    /// and a plan that packets a range no stored packet covers yet renders
    /// those turns as cards — the cheap, safe form — rather than losing
    /// them (no-cut invariant). The caller normally writes the packet
    /// before projecting (`Agent`'s incremental compaction, §4), so that
    /// fallback is a transient.
    fn derive_with_plan_tagged(
        &self,
        plan: Option<&WorkingSetPlan>,
    ) -> Vec<(String, Message, bool, bool)> {
        self.derive_with_plan_indexed(plan).0
    }

    /// The projection and how many of its trailing messages are the open
    /// turn's own, from the one pass over the ledger that built it.
    fn derive_with_plan_indexed(
        &self,
        plan: Option<&WorkingSetPlan>,
    ) -> (Vec<(String, Message, bool, bool)>, usize) {
        if let Some(plan) = plan.filter(|plan| plan.selected_records.is_some()) {
            let open_len = self
                .current_turn_index()
                .turns
                .last()
                .filter(|turn| !turn.closed)
                .map_or(0, |turn| turn.current_verbatim().len());
            return (self.derive_selected(plan), open_len);
        }
        let mut open_len = 0usize;
        let (_, position_owned, reset_summary) = self.reset_boundary();
        let position: HashMap<&str, usize> = position_owned
            .iter()
            .map(|(id, pos)| (id.as_str(), *pos))
            .collect();

        let mut index = TurnIndex::from_log(self);
        // Rendering needs a card line for every closed turn the plan put at
        // `Card`; the token counts on a provisional card are irrelevant
        // here (the plan was costed by the caller), so they are left at 0.
        index.ensure_cards(&|_| 0);
        let mut out: Vec<(String, Message, bool, bool)> = Vec::new();
        if let Some(summary) = &reset_summary {
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
        let packet = plan
            .and_then(|p| p.packet_range.as_ref())
            .and_then(|(first, last)| self.packet_for(first, last));
        if let Some(packet) = &packet {
            // Name the turns it stands in for, so a detail can be reopened
            // with recall({ turn }).
            let number = |id: &str| index.turns.iter().position(|t| t.id == id).map(|i| i + 1);
            let covers = match (number(&packet.first_turn_id), number(&packet.last_turn_id)) {
                (Some(first), Some(last)) => {
                    format!("Summary of turns {first}-{last}; recall({{ turn: N }}) reopens one.\n")
                }
                _ => String::new(),
            };
            let summary_msg = Message::user_text(format!(
                "<context_summary>\n{covers}{}\n</context_summary>",
                packet.summary
            ));
            out.push((String::new(), summary_msg, true, false));
        }

        let mut card_lines: Vec<String> = Vec::new();
        let mut open_turn_at: Option<usize> = None;
        for (turn_number, turn) in index.turns.iter().enumerate() {
            let turn_pos = position.get(turn.id.as_str()).copied().unwrap_or(0);
            // A turn still open across a reset keeps its directive and its
            // steps after the reset (`behind_reset` is false for it).
            if turn.behind_reset {
                continue;
            }
            if !turn.closed {
                open_turn_at = Some(out.len());
                for message in turn.current_verbatim() {
                    out.push((turn.id.clone(), message, false, false));
                    open_len += 1;
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
                        if packet.is_some() {
                            Fidelity::Packet
                        } else {
                            // The plan asked for a packet nobody has
                            // written yet: cards, never nothing.
                            Fidelity::Card
                        }
                    } else {
                        // The plan never classified it (a stale plan
                        // against a longer chain) — render the cheap, safe
                        // form rather than silently losing it (no-cut
                        // invariant); every closed turn has a card here.
                        Fidelity::Card
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
                    // Represented by the packet summary pushed above.
                }
            }
        }
        if !card_lines.is_empty() {
            let block = format!("<turns>\n{}\n</turns>", card_lines.join("\n"));
            // After the Full turns, just before the open turn: a new card line
            // changes this block every turn, and everything ahead of it (the
            // summary and the Full records) stays byte-identical, so the
            // provider's prefix cache keeps serving it.
            let insert_at = open_turn_at.unwrap_or(out.len());
            out.insert(
                insert_at,
                (String::new(), Message::user_text(block), true, false),
            );
        }
        (out, open_len)
    }

    /// Reconstruct only addressed closed turns plus the active turn. The host
    /// has already validated source membership against the selected leaf.
    fn derive_selected(&self, plan: &WorkingSetPlan) -> Vec<(String, Message, bool, bool)> {
        self.derive_selected_at(plan, self.tail_id.as_deref())
    }

    fn derive_selected_at(
        &self,
        plan: &WorkingSetPlan,
        leaf: Option<&str>,
    ) -> Vec<(String, Message, bool, bool)> {
        let mut out = Vec::new();
        for (turn_id, closing_id) in plan.selected_records.iter().flatten() {
            let Some(fidelity) = plan
                .per_turn
                .iter()
                .find_map(|(id, fidelity)| (id == turn_id).then_some(fidelity))
            else {
                continue;
            };
            let mut entries = Vec::new();
            let mut cursor = Some(closing_id.as_str());
            while let Some(id) = cursor {
                let Some(entry) = self
                    .by_id
                    .get(id)
                    .and_then(|index| self.entries.get(*index))
                else {
                    break;
                };
                entries.push(entry);
                if entry.id == *turn_id {
                    break;
                }
                if entries.len() >= 512 {
                    break;
                }
                cursor = entry.parent_id.as_deref();
            }
            if entries.last().is_none_or(|entry| entry.id != *turn_id) {
                continue;
            }
            entries.reverse();
            let index = TurnIndex::from_entries_resolving(entries, &|body| self.object_text(body));
            let Some(turn) = index.turn_by_id(turn_id).filter(|turn| turn.closed) else {
                continue;
            };
            match fidelity {
                Fidelity::Full => out.extend(
                    turn.full_record()
                        .into_iter()
                        .map(|message| (turn_id.clone(), message, false, false)),
                ),
                Fidelity::Card => {
                    if let Some(card) = &turn.card {
                        out.push((
                            turn_id.clone(),
                            Message::user_text(card.addressed_message()),
                            true,
                            false,
                        ));
                    }
                }
                Fidelity::Packet => {}
            }
        }
        let current = self.current_turn_index_at(leaf);
        if let Some(turn) = current
            .turns
            .last()
            .filter(|turn| !turn.closed && !turn.behind_reset)
        {
            out.extend(
                turn.current_verbatim()
                    .into_iter()
                    .map(|message| (turn.id.clone(), message, false, false)),
            );
        }
        out
    }

    /// Checked addressed projection. A missing or mismatched source is an
    /// error, never an apparently successful projection with history omitted.
    /// The broker must separately authorize the selected leaf and sources.
    pub fn derive_selected_checked(
        &self,
        plan: &WorkingSetPlan,
    ) -> Result<Vec<Message>, SessionError> {
        self.derive_selected_checked_at(plan, self.tail_id.as_deref())
    }

    fn derive_selected_checked_at(
        &self,
        plan: &WorkingSetPlan,
        leaf: Option<&str>,
    ) -> Result<Vec<Message>, SessionError> {
        let invalid = || SessionError::InvalidSelection;
        let sources = plan.selected_records.as_ref().ok_or_else(invalid)?;
        if plan.packet_range.is_some() || sources.len() != plan.per_turn.len() {
            return Err(invalid());
        }
        let mut seen = std::collections::HashSet::new();
        for (turn_id, closing_id) in sources {
            if !seen.insert(turn_id) {
                return Err(invalid());
            }
            let fidelity = plan
                .per_turn
                .iter()
                .find_map(|(id, fidelity)| (id == turn_id).then_some(fidelity))
                .ok_or_else(invalid)?;
            if matches!(fidelity, Fidelity::Packet) {
                return Err(invalid());
            }
            let closing = self
                .by_id
                .get(closing_id)
                .and_then(|index| self.entries.get(*index))
                .ok_or_else(invalid)?;
            if !matches!(&closing.payload, EntryPayload::TurnCard(record) if record.turn_id == *turn_id)
            {
                return Err(invalid());
            }
            let mut cursor = Some(closing_id.as_str());
            let mut count = 0;
            let mut found = false;
            while let Some(id) = cursor {
                let entry = self
                    .by_id
                    .get(id)
                    .and_then(|index| self.entries.get(*index))
                    .ok_or_else(invalid)?;
                count += 1;
                if entry.id == *turn_id {
                    found = true;
                    break;
                }
                if count >= 512 {
                    break;
                }
                cursor = entry.parent_id.as_deref();
            }
            if !found {
                return Err(invalid());
            }
        }
        Ok(self
            .derive_selected_at(plan, leaf)
            .into_iter()
            .map(|(_, message, _, _)| message)
            .collect())
    }

    /// Record the checked projection before dispatch. The captured leaf
    /// prevents subsequent tool results or future turns from changing replay.
    pub fn append_context_selection(
        &mut self,
        plan: WorkingSetPlan,
    ) -> Result<Entry, SessionError> {
        self.derive_selected_checked(&plan)?;
        let leaf_id = self.tail_id.clone().ok_or(SessionError::InvalidSelection)?;
        let record = crate::types::ContextSelectionRecord {
            policy_version: 1,
            leaf_id,
            plan,
        };
        self.append(Entry::new(
            self.tail_id.clone(),
            EntryPayload::ContextSelection(record),
        ))
    }

    /// Reconstruct the exact message projection captured by a selection entry.
    /// Prefix/tool interfaces remain in the turn's capability binding.
    pub fn replay_context_selection(&self, entry_id: &str) -> Result<Vec<Message>, SessionError> {
        let entry = self
            .by_id
            .get(entry_id)
            .and_then(|index| self.entries.get(*index))
            .ok_or(SessionError::InvalidSelection)?;
        let EntryPayload::ContextSelection(record) = &entry.payload else {
            return Err(SessionError::InvalidSelection);
        };
        if record.policy_version != 1 || entry.parent_id.as_deref() != Some(record.leaf_id.as_str())
        {
            return Err(SessionError::InvalidSelection);
        }
        self.derive_selected_checked_at(&record.plan, Some(&record.leaf_id))
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

    /// `derive_with_plan`'s projection alongside the index within it where
    /// the open turn's own directive sits — `messages.len()` (an
    /// out-of-bounds sentinel, safe as a no-op) when the chain has no open
    /// turn. What a request assembler needs to attach the per-turn tail to
    /// the turn's directive specifically (docs/design/68-context-engine.md
    /// §6/§7): the directive is always the first message of the open
    /// turn's own `current_verbatim` block, which `derive_with_plan`
    /// places last, but a step within the turn appends more messages
    /// (tool results, control nudges) after it — the tail must ride on the
    /// directive every step, not on whatever the last message happens to
    /// be, or an earlier-sent message would silently change shape between
    /// requests.
    pub fn derive_with_plan_and_directive(&self, plan: &WorkingSetPlan) -> (Vec<Message>, usize) {
        let (tagged, open_len) = self.derive_with_plan_indexed(Some(plan));
        let messages: Vec<Message> = tagged.into_iter().map(|(_, m, _, _)| m).collect();
        let directive_at = if open_len == 0 {
            messages.len()
        } else {
            messages.len().saturating_sub(open_len)
        };
        (messages, directive_at)
    }

    /// Appends the packet a `WorkingSetPlan` asked for (its `packet_range`,
    /// `first_turn_id..=last_turn_id`), produced by INCREMENTAL compaction
    /// (§4): summarizing the turns' CARDS (never raw history). `model` is
    /// the model whose plan asked for it. The packet is keyed by its range
    /// and is reused by any later plan — under any model — that asks for
    /// exactly that range; it hides nothing from a plan that does not.
    pub fn append_incremental_compaction(
        &mut self,
        first_turn_id: &str,
        last_turn_id: &str,
        model: &str,
        summary: String,
        tokens_before: u64,
    ) -> Result<(), SessionError> {
        self.append_packet(first_turn_id, last_turn_id, model, summary, tokens_before)?;
        Ok(())
    }

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
                    author_id: record.meta.as_ref().and_then(|meta| meta.author_id.clone()),
                    author_name: record
                        .meta
                        .as_ref()
                        .and_then(|meta| meta.author_name.clone()),
                    attachments: record
                        .meta
                        .as_ref()
                        .map(|meta| meta.attachments.clone())
                        .unwrap_or_default(),
                    typed: record.meta.as_ref().and_then(|meta| meta.typed.clone()),
                }),
                EntryPayload::Compaction(c) => Some(TranscriptMessage {
                    entry_id: entry.id.clone(),
                    message: Message::user_text(format!(
                        "<context_summary>\n{}\n</context_summary>",
                        c.summary
                    )),
                    control: None,
                    context: true,
                    author_id: None,
                    author_name: None,
                    attachments: Vec::new(),
                    typed: None,
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
            .map(|item| item.as_typed())
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
    /// The still-open turn's own verbatim messages, respecting the reset
    /// boundary (docs/design/68-context-engine.md §4): after a
    /// reset-with-handoff, everything before the reset entry is invisible
    /// to the model even though the open turn technically started before
    /// it, so planning must size its reserve against what will actually be
    /// sent — never against text the reset already discarded. Empty when
    /// there is no open turn, or the open turn itself predates the
    /// boundary.
    pub fn open_turn_verbatim(&self) -> Vec<Message> {
        let index = self.current_turn_index();
        let Some(turn) = index
            .turns
            .last()
            .filter(|turn| !turn.closed && !turn.behind_reset)
        else {
            return Vec::new();
        };
        turn.current_verbatim()
    }

    pub fn latest_reading(&self) -> Option<crate::turns::ReadingKey> {
        self.active_entries_rev()
            .find_map(|entry| match &entry.payload {
                EntryPayload::Intent(record) => Some(crate::turns::ReadingKey::from_record(record)),
                _ => None,
            })
    }

    /// The session-derived tail sections for one request. `plan` is the
    /// working-set plan the request will be projected with: the
    /// conversation thread lists only directives that plan leaves out of
    /// the projection (packeted, or behind a reset), so a directive the
    /// model already sees — verbatim in a `Full` turn, as a card's
    /// `asked:` line, or in the open turn — is never restated (one source
    /// per fact, docs/design/68-context-engine.md §6). With `None` every
    /// closed turn is projected at `Full`, so the thread is empty unless a
    /// reset hid something.
    pub fn tail_sections(&self, plan: Option<&WorkingSetPlan>) -> TailSections {
        TailSections {
            intent: self.tail_intent(),
            work_contract: self.tail_work_contract(),
            thread: self.tail_conversation_thread(plan),
            workspace: self.tail_workspace_delta(),
        }
    }

    /// The `data.section = "workspace_delta"` activity this turn recorded.
    ///
    /// The value used by the `workspace_delta` activity's `data` key.
    pub const WORKSPACE_DELTA_SECTION: &'static str = "workspace_delta";

    /// The value used by the turn-context activity's `data` key: the host's
    /// `<turn_context>` text in `detail` and its `<stance>` text in
    /// `data["stance"]`, recorded before the turn's first request.
    pub const TURN_CONTEXT_SECTION: &'static str = "turn_context";

    /// The workspace delta recorded for the current turn, tagged — only
    /// when the turn's reading asked for it (`working` / `full`), and only
    /// the activity written after this turn's intent entry, so a previous
    /// turn's delta is never shown as current.
    fn tail_workspace_delta(&self) -> Option<String> {
        let chain = self.chain_to_root();
        let intent_position = chain
            .iter()
            .rposition(|entry| matches!(entry.payload, EntryPayload::Intent(_)))?;
        let wants = match &chain[intent_position].payload {
            EntryPayload::Intent(record) => {
                crate::turns::ReadingKey::from_record(record).wants_workspace_delta()
            }
            _ => false,
        };
        if !wants {
            return None;
        }
        let delta = chain[intent_position + 1..]
            .iter()
            .rev()
            .find_map(|entry| match &entry.payload {
                EntryPayload::Activity(activity)
                    if activity.data.get("section").map(String::as_str)
                        == Some(Self::WORKSPACE_DELTA_SECTION) =>
                {
                    activity.detail.clone()
                }
                _ => None,
            })?;
        let trimmed = delta.trim();
        if trimmed.is_empty() {
            return None;
        }
        // Bounded where it is written (`checkpoints::delta_summary`'s byte
        // budget and excerpt cap), never cut here: a byte truncation panics
        // inside a multi-byte character, and invariant 36 forbids a blind cut.
        Some(format!("<workspace_delta>\n{trimmed}\n</workspace_delta>"))
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
        Some(format!("<intent>\n{note}\n</intent>"))
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
        Some(context)
    }

    /// The directive texts the projection under `plan` already carries:
    /// the open turn's, every `Full` or `Card` turn's, and — with no plan —
    /// every turn's at or after the reset boundary.
    fn covered_directives(
        &self,
        plan: Option<&WorkingSetPlan>,
    ) -> std::collections::HashSet<String> {
        let fidelity_of: HashMap<&str, Fidelity> = plan
            .map(|p| p.per_turn.iter().map(|(id, f)| (id.as_str(), *f)).collect())
            .unwrap_or_default();
        TurnIndex::from_log(self)
            .turns
            .iter()
            .filter(|turn| !turn.behind_reset)
            .filter(|turn| {
                !turn.closed
                    || plan.is_none()
                    || matches!(
                        fidelity_of.get(turn.id.as_str()),
                        Some(Fidelity::Full | Fidelity::Card)
                    )
            })
            .map(|turn| turn.directive.text_content().trim().to_string())
            .collect()
    }

    /// The multi-turn directive timeline, tagged, restricted to directives
    /// the projection under `plan` does not already carry — one source per
    /// fact (docs/design/68-context-engine.md §6): a directive the model
    /// sees in a `Full` turn, on a card line, or in the open turn needs no
    /// restating here; only packeted or reset-hidden ones do. `None` when
    /// there is no active goal spanning multiple turns, a managed work
    /// contract already covers progress, or nothing is left out.
    fn tail_conversation_thread(&self, plan: Option<&WorkingSetPlan>) -> Option<String> {
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

        let verbatim: std::collections::HashSet<String> = self.covered_directives(plan);
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

        // Only a goal a person stated is presented as the objective. The
        // first message of a conversation is not one by default: rendering
        // "hi" as the primary objective of every later turn misdirects the
        // model more than it orients it.
        let mut thread = format!("<conversation_thread revision=\"{}\">\n", goal.revision);
        if goal.explicit {
            thread.push_str(&format!("Primary objective: {}\n", goal.objective.trim()));
        }
        thread.push_str("User request timeline across turns:\n");
        for (num, req) in filtered {
            let words: Vec<&str> = req.split_whitespace().collect();
            let preview = if words.len() > 40 {
                format!("{}...", words[..40].join(" "))
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
    /// `exclude_tail` most recent entries.
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
        vak_config::scope::AgentScope::new(home).sessions_dir(cwd)
    }

    /// Where a new session's ledger goes. Creating a session for `cwd`
    /// opens it as a workspace, so the folder is bound to its space first
    /// (`vak_config::spaces::bind`); if that fails, the path keeps its
    /// `unbound-` key and `SessionLog::create` refuses it with the reason.
    pub fn new_session_file(home: &Path, cwd: &Path, session_id: &str) -> PathBuf {
        let _ = vak_config::spaces::bind(cwd);
        Self::existing_session_file(home, cwd, session_id)
    }

    /// Where an existing session's ledger is; never binds a folder.
    pub fn existing_session_file(home: &Path, cwd: &Path, session_id: &str) -> PathBuf {
        vak_config::scope::session_ledger(&Self::sessions_dir(home, cwd), session_id)
    }
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// The writer lock lives exactly as long as the handle (its release
    /// with a duplicate descriptor outstanding is tested in vak-storage).
    #[test]
    fn opening_a_missing_ledger_fails_and_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent");
        assert!(SessionLog::open(path.clone()).is_err());
        assert!(
            !path.exists(),
            "an open never makes the ledger it did not find"
        );
    }

    #[test]
    fn the_lock_lasts_exactly_as_long_as_the_handle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session");
        std::fs::create_dir_all(&path).unwrap();

        let log = SessionLog::open(path.clone()).unwrap();
        assert!(matches!(
            SessionLog::open(path.clone()),
            Err(SessionError::Locked(_))
        ));

        drop(log);
        SessionLog::open(path).expect("the dropped handle released its lock");
    }
}
