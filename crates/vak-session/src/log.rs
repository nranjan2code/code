use crate::types::{Entry, EntryPayload, MessageRecord, SessionError, SessionHeader};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use vak_llm::Message;

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
        validate_id(&header.project_id)?;
        validate_id(&header.session_id)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        file.try_lock()
            .map_err(|_| SessionError::Locked(path.clone()))?;
        if file.metadata()?.len() != 0 {
            return Err(SessionError::Exists(path));
        }
        let mut log = Self {
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
    pub fn create_at(
        home: &Path,
        project_id: &str,
        header: SessionHeader,
    ) -> Result<Self, SessionError> {
        if header.project_id != project_id {
            return Err(SessionError::InvalidIdentifier(
                "header project does not match path".into(),
            ));
        }
        Self::create(
            SessionPath::new_session_file(home, project_id, &header.session_id)?,
            header,
        )
    }
    pub fn open(path: PathBuf) -> Result<Self, SessionError> {
        let file = OpenOptions::new().append(true).open(&path)?;
        file.try_lock()
            .map_err(|_| SessionError::Locked(path.clone()))?;
        let bytes = std::fs::read(&path)?;
        let mut log = Self {
            path,
            file,
            entries: Vec::new(),
            by_id: HashMap::new(),
            tail_id: None,
            warnings: Vec::new(),
        };
        let line_count = bytes.split(|byte| *byte == b'\n').count();
        for (line_no, raw) in bytes.split(|byte| *byte == b'\n').enumerate() {
            let line_no = line_no + 1;
            let line = std::str::from_utf8(raw).map_err(|e| SessionError::Corrupt {
                line: line_no,
                message: e.to_string(),
            })?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Entry>(line) {
                Ok(entry) => {
                    if entry
                        .parent_id
                        .as_ref()
                        .is_some_and(|id| !log.by_id.contains_key(id))
                    {
                        return Err(SessionError::Corrupt {
                            line: line_no,
                            message: "parent entry not found".into(),
                        });
                    }
                    log.by_id.insert(entry.id.clone(), log.entries.len());
                    log.tail_id = Some(entry.id.clone());
                    log.entries.push(entry);
                }
                Err(error) if line_no == line_count => log.warnings.push(format!(
                    "ignored unparseable final entry at line {line_no}: {error}"
                )),
                Err(error) => {
                    return Err(SessionError::Corrupt {
                        line: line_no,
                        message: error.to_string(),
                    });
                }
            }
        }
        if !matches!(
            log.entries.first().map(|e| &e.payload),
            Some(EntryPayload::Header(_))
        ) {
            return Err(SessionError::MissingHeader);
        }
        Ok(log)
    }
    pub fn open_at(home: &Path, project_id: &str, session_id: &str) -> Result<Self, SessionError> {
        validate_id(project_id)?;
        validate_id(session_id)?;
        Self::open(SessionPath::new_session_file(home, project_id, session_id)?)
    }
    pub fn append(&mut self, entry: Entry) -> Result<Entry, SessionError> {
        if let Some(parent) = &entry.parent_id
            && !self.by_id.contains_key(parent)
        {
            return Err(SessionError::Corrupt {
                line: 0,
                message: format!("parent entry {parent} not found"),
            });
        }
        let line = serde_json::to_string(&entry).map_err(|e| SessionError::Corrupt {
            line: 0,
            message: e.to_string(),
        })?;
        writeln!(self.file, "{line}")?;
        self.file.flush()?;
        self.file.sync_data()?;
        self.by_id.insert(entry.id.clone(), self.entries.len());
        self.tail_id = Some(entry.id.clone());
        self.entries.push(entry.clone());
        Ok(entry)
    }
    pub fn append_message(&mut self, record: MessageRecord) -> Result<Entry, SessionError> {
        self.append(Entry::new(
            self.tail_id.clone(),
            EntryPayload::Message(record),
        ))
    }
    pub fn branch_at(&mut self, entry_id: &str) -> Result<(), SessionError> {
        if !self.by_id.contains_key(entry_id) {
            return Err(SessionError::Corrupt {
                line: 0,
                message: format!("unknown entry {entry_id}"),
            });
        }
        self.tail_id = Some(entry_id.to_owned());
        Ok(())
    }
    pub fn derive_messages(&self) -> Vec<Message> {
        self.chain_to_root()
            .into_iter()
            .filter_map(|e| match &e.payload {
                EntryPayload::Message(record) => Some(record.message.clone()),
                _ => None,
            })
            .collect()
    }
    pub fn chain_to_root(&self) -> Vec<&Entry> {
        let mut result = Vec::new();
        let mut cursor = self.tail_id.clone();
        while let Some(id) = cursor {
            let Some(index) = self.by_id.get(&id) else {
                break;
            };
            let entry = &self.entries[*index];
            cursor = entry.parent_id.clone();
            result.push(entry);
        }
        result.reverse();
        result
    }
    pub fn header(&self) -> Option<&SessionHeader> {
        self.entries.iter().find_map(|e| match &e.payload {
            EntryPayload::Header(h) => Some(h),
            _ => None,
        })
    }
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn tail_id(&self) -> Option<&str> {
        self.tail_id.as_deref()
    }
}
fn validate_id(value: &str) -> Result<(), SessionError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
    {
        return Err(SessionError::InvalidIdentifier(value.into()));
    }
    Ok(())
}
pub struct SessionPath;
impl SessionPath {
    pub fn sessions_dir(home: &Path, project_id: &str) -> Result<PathBuf, SessionError> {
        validate_id(project_id)?;
        Ok(home.join("sessions").join(project_id))
    }
    pub fn new_session_file(
        home: &Path,
        project_id: &str,
        session_id: &str,
    ) -> Result<PathBuf, SessionError> {
        validate_id(session_id)?;
        Ok(Self::sessions_dir(home, project_id)?.join(format!("{session_id}.jsonl")))
    }
}
