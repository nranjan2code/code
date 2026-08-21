use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use vak_llm::Message;

use crate::types::{Entry, EntryPayload, MessageMeta, MessageRecord, SessionError, SessionHeader};

pub struct SessionLog {
    path: PathBuf,
    file: File,
    entries: Vec<Entry>,
    by_id: HashMap<String, usize>,
    tail_id: Option<String>,
}

impl SessionLog {
    pub fn create(path: PathBuf, header: SessionHeader) -> Result<Self, SessionError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut log = SessionLog {
            path,
            file,
            entries: Vec::new(),
            by_id: HashMap::new(),
            tail_id: None,
        };
        log.append(Entry::new(None, EntryPayload::Header(header)))?;
        Ok(log)
    }

    pub fn open(path: PathBuf) -> Result<Self, SessionError> {
        let file = OpenOptions::new().append(true).open(&path)?;
        let reader = BufReader::new(File::open(&path)?);
        let mut entries = Vec::new();
        let mut by_id = HashMap::new();
        for (i, line) in reader.lines().enumerate() {
            let line = line.map_err(|e| SessionError::Corrupt {
                line: i + 1,
                message: e.to_string(),
            })?;
            if line.trim().is_empty() {
                continue;
            }
            let entry: Entry = serde_json::from_str(&line).map_err(|e| SessionError::Corrupt {
                line: i + 1,
                message: e.to_string(),
            })?;
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
        })
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

    pub fn derive_messages(&self) -> Vec<Message> {
        let mut out: Vec<(String, Message)> = Vec::new();
        for entry in self.chain_to_root() {
            match &entry.payload {
                EntryPayload::Message(record) => {
                    out.push((entry.id.clone(), record.message.clone()));
                }
                EntryPayload::Compaction(c) => {
                    let keep_from = out
                        .iter()
                        .position(|(id, _)| id == &c.first_kept_entry_id)
                        .unwrap_or(out.len());
                    out.drain(..keep_from);
                    let summary_msg = Message::user_text(format!(
                        "<context_summary>\n{}\n</context_summary>",
                        c.summary
                    ));
                    out.insert(0, (entry.id.clone(), summary_msg));
                }
                EntryPayload::Header(_) => {}
            }
        }
        out.into_iter().map(|(_, m)| m).collect()
    }

    pub fn total_usage(&self) -> vak_llm::Usage {
        let mut total = vak_llm::Usage::default();
        for e in &self.entries {
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

fn hash_cwd(cwd: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    cwd.hash(&mut h);
    format!("{:016x}", h.finish())
}
