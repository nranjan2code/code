//! Durable Runtime-owned control-plane services.
//!
//! Every mutation is transactional SQLite state (or an append-only audit/blob
//! operation). State is stored only in the Runtime SQLite authority.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use thiserror::Error;
use vak_storage::{AuditEvent, AuditWriter, BlobStore, StateStore, StorageError};

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("storage: {0}")]
    Storage(#[from] StorageError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{kind} {id} not found")]
    NotFound { kind: &'static str, id: String },
    #[error("invalid state transition: {from} -> {to}")]
    InvalidTransition { from: String, to: String },
}
pub type Result<T> = std::result::Result<T, ServiceError>;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TaskRecord {
    pub id: String,
    pub project_id: Option<String>,
    pub spec: Value,
    pub status: String,
    pub updated_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ApprovalRecord {
    pub id: String,
    pub run_id: String,
    pub status: String,
    pub request: Value,
    pub response: Option<Value>,
    pub created_at: String,
    pub resolved_at: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct InboxRecord {
    pub id: String,
    pub channel: String,
    pub external_id: Option<String>,
    pub payload: Value,
    pub created_at: String,
    pub acknowledged_at: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DeliveryRecord {
    pub id: String,
    pub channel: String,
    pub payload: Value,
    pub status: String,
    pub attempts: u32,
    pub next_attempt_at: Option<String>,
    pub updated_at: String,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BindingRecord {
    pub id: String,
    pub channel: String,
    pub external_key: String,
    pub target: Value,
    pub created_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MemoryRecord {
    pub id: String,
    pub project_id: Option<String>,
    pub scope: String,
    pub kind: String,
    pub tag: String,
    pub text: String,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SkillRecord {
    pub id: String,
    pub project_id: Option<String>,
    pub name: String,
    pub description: String,
    pub body_digest: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CheckpointRecord {
    pub id: String,
    pub session_id: String,
    pub manifest_digest: String,
    pub created_at: String,
    pub label: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UsageRecord {
    pub id: String,
    pub run_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_cents: Option<i64>,
    pub created_at: String,
}

pub struct Services {
    pub state: StateStore,
    pub blobs: BlobStore,
    pub audit: AuditWriter,
}
impl Services {
    pub fn open(data_home: impl AsRef<Path>) -> Result<Self> {
        let home = data_home.as_ref();
        std::fs::create_dir_all(home)?;
        Ok(Self {
            state: StateStore::open(home.join("state.db"))?,
            blobs: BlobStore::open(home.join("blobs"))?,
            audit: AuditWriter::open(home.join("audit/operations.jsonl")),
        })
    }
    pub fn new(state: StateStore, blobs: BlobStore, audit: AuditWriter) -> Self {
        Self {
            state,
            blobs,
            audit,
        }
    }

    pub fn create_task(
        &self,
        id: &str,
        project_id: Option<&str>,
        spec: &Value,
        status: &str,
        now: &str,
    ) -> Result<TaskRecord> {
        self.state.connection().execute(
            "INSERT INTO tasks(id,project_id,spec_json,status,updated_at) VALUES(?1,?2,?3,?4,?5)",
            params![id, project_id, serde_json::to_string(spec)?, status, now],
        )?;
        Ok(TaskRecord {
            id: id.into(),
            project_id: project_id.map(str::to_owned),
            spec: spec.clone(),
            status: status.into(),
            updated_at: now.into(),
        })
    }
    pub fn get_task(&self, id: &str) -> Result<Option<TaskRecord>> {
        Ok(self
            .state
            .connection()
            .query_row(
                "SELECT id,project_id,spec_json,status,updated_at FROM tasks WHERE id=?1",
                [id],
                task_row,
            )
            .optional()?)
    }
    pub fn list_tasks(&self, project_id: Option<&str>) -> Result<Vec<TaskRecord>> {
        let mut s=self.state.connection().prepare("SELECT id,project_id,spec_json,status,updated_at FROM tasks WHERE (?1 IS NULL OR project_id=?1) ORDER BY updated_at")?;
        Ok(s.query_map([project_id], task_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn update_task(
        &self,
        id: &str,
        project_id: Option<Option<&str>>,
        spec: Option<&Value>,
        status: Option<&str>,
        now: &str,
    ) -> Result<TaskRecord> {
        let old = self.get_task(id)?.ok_or_else(|| ServiceError::NotFound {
            kind: "task",
            id: id.into(),
        })?;
        let pid = project_id
            .flatten()
            .map(str::to_owned)
            .or(old.project_id.clone());
        let next_spec = spec.cloned().unwrap_or(old.spec);
        let next_status = status.unwrap_or(&old.status);
        self.state.connection().execute(
            "UPDATE tasks SET project_id=?2,spec_json=?3,status=?4,updated_at=?5 WHERE id=?1",
            params![
                id,
                pid,
                serde_json::to_string(&next_spec)?,
                next_status,
                now
            ],
        )?;
        self.get_task(id)?.ok_or_else(|| ServiceError::NotFound {
            kind: "task",
            id: id.into(),
        })
    }
    pub fn delete_task(&self, id: &str) -> Result<bool> {
        Ok(self
            .state
            .connection()
            .execute("DELETE FROM tasks WHERE id=?1", [id])?
            == 1)
    }

    pub fn create_approval(
        &self,
        id: &str,
        run_id: &str,
        request: &Value,
        now: &str,
    ) -> Result<ApprovalRecord> {
        self.state.connection().execute("INSERT INTO approvals(id,run_id,status,request_json,created_at) VALUES(?1,?2,'pending',?3,?4)", params![id,run_id,serde_json::to_string(request)?,now])?;
        self.get_approval(id)?
            .ok_or_else(|| ServiceError::NotFound {
                kind: "approval",
                id: id.into(),
            })
    }
    pub fn get_approval(&self, id: &str) -> Result<Option<ApprovalRecord>> {
        Ok(self.state.connection().query_row("SELECT id,run_id,status,request_json,response_json,created_at,resolved_at FROM approvals WHERE id=?1", [id], approval_row).optional()?)
    }
    pub fn pending_approvals(&self) -> Result<Vec<ApprovalRecord>> {
        let mut s=self.state.connection().prepare("SELECT id,run_id,status,request_json,response_json,created_at,resolved_at FROM approvals WHERE status='pending' ORDER BY created_at")?;
        Ok(s.query_map([], approval_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn resolve_approval(
        &self,
        id: &str,
        allow: bool,
        response: &Value,
        now: &str,
    ) -> Result<ApprovalRecord> {
        let changed=self.state.connection().execute("UPDATE approvals SET status=?2,response_json=?3,resolved_at=?4 WHERE id=?1 AND status='pending'", params![id,if allow{"allowed"}else{"denied"},serde_json::to_string(response)?,now])?;
        if changed == 0 {
            return Err(ServiceError::InvalidTransition {
                from: "resolved_or_missing".into(),
                to: "resolved".into(),
            });
        }
        self.get_approval(id)?
            .ok_or_else(|| ServiceError::NotFound {
                kind: "approval",
                id: id.into(),
            })
    }

    pub fn insert_inbox(
        &self,
        id: &str,
        channel: &str,
        external_id: Option<&str>,
        payload: &Value,
        now: &str,
    ) -> Result<InboxRecord> {
        self.state.connection().execute("INSERT INTO inbox(id,channel,external_id,payload_json,created_at) VALUES(?1,?2,?3,?4,?5)", params![id,channel,external_id,serde_json::to_string(payload)?,now])?;
        self.get_inbox(id)?.ok_or_else(|| ServiceError::NotFound {
            kind: "inbox",
            id: id.into(),
        })
    }
    pub fn get_inbox(&self, id: &str) -> Result<Option<InboxRecord>> {
        Ok(self.state.connection().query_row("SELECT id,channel,external_id,payload_json,created_at,acknowledged_at FROM inbox WHERE id=?1",[id],inbox_row).optional()?)
    }
    pub fn list_inbox(&self, unread: bool) -> Result<Vec<InboxRecord>> {
        let mut s=self.state.connection().prepare("SELECT id,channel,external_id,payload_json,created_at,acknowledged_at FROM inbox WHERE (?1=0 OR acknowledged_at IS NULL) ORDER BY created_at")?;
        Ok(s.query_map([unread as i64], inbox_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn acknowledge_inbox(&self, id: &str, now: &str) -> Result<bool> {
        Ok(self.state.connection().execute(
            "UPDATE inbox SET acknowledged_at=?2 WHERE id=?1 AND acknowledged_at IS NULL",
            params![id, now],
        )? == 1)
    }

    pub fn enqueue_delivery(
        &self,
        id: &str,
        channel: &str,
        payload: &Value,
        now: &str,
    ) -> Result<DeliveryRecord> {
        self.state.connection().execute("INSERT INTO delivery_jobs(id,channel,payload_json,status,updated_at) VALUES(?1,?2,?3,'pending',?4)",params![id,channel,serde_json::to_string(payload)?,now])?;
        self.get_delivery(id)?
            .ok_or_else(|| ServiceError::NotFound {
                kind: "delivery",
                id: id.into(),
            })
    }
    pub fn get_delivery(&self, id: &str) -> Result<Option<DeliveryRecord>> {
        Ok(self.state.connection().query_row("SELECT id,channel,payload_json,status,attempts,next_attempt_at,updated_at,error FROM delivery_jobs WHERE id=?1",[id],delivery_row).optional()?)
    }
    pub fn claim_delivery(&self, id: &str, now: &str) -> Result<Option<DeliveryRecord>> {
        self.state.connection().execute("UPDATE delivery_jobs SET status='sending',attempts=attempts+1,updated_at=?2 WHERE id=?1 AND status IN ('pending','retry')",params![id,now])?;
        self.get_delivery(id)
    }
    pub fn mark_delivery_retry(
        &self,
        id: &str,
        next: &str,
        error: &str,
        now: &str,
    ) -> Result<DeliveryRecord> {
        self.state.connection().execute("UPDATE delivery_jobs SET status='retry',next_attempt_at=?2,error=?3,updated_at=?4 WHERE id=?1 AND status='sending'",params![id,next,error,now])?;
        self.get_delivery(id)?
            .ok_or_else(|| ServiceError::NotFound {
                kind: "delivery",
                id: id.into(),
            })
    }
    pub fn mark_delivery_delivered(&self, id: &str, now: &str) -> Result<DeliveryRecord> {
        self.state.connection().execute("UPDATE delivery_jobs SET status='delivered',next_attempt_at=NULL,error=NULL,updated_at=?2 WHERE id=?1 AND status='sending'",params![id,now])?;
        self.get_delivery(id)?
            .ok_or_else(|| ServiceError::NotFound {
                kind: "delivery",
                id: id.into(),
            })
    }

    pub fn upsert_binding(
        &self,
        id: &str,
        channel: &str,
        key: &str,
        target: &Value,
        now: &str,
    ) -> Result<BindingRecord> {
        self.state.connection().execute("INSERT INTO bindings(id,channel,external_key,target_json,created_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(channel,external_key) DO UPDATE SET id=excluded.id,target_json=excluded.target_json",params![id,channel,key,serde_json::to_string(target)?,now])?;
        self.get_binding(channel, key)?
            .ok_or_else(|| ServiceError::NotFound {
                kind: "binding",
                id: id.into(),
            })
    }
    pub fn get_binding(&self, channel: &str, key: &str) -> Result<Option<BindingRecord>> {
        Ok(self.state.connection().query_row("SELECT id,channel,external_key,target_json,created_at FROM bindings WHERE channel=?1 AND external_key=?2",params![channel,key],binding_row).optional()?)
    }
    pub fn delete_binding(&self, channel: &str, key: &str) -> Result<bool> {
        Ok(self.state.connection().execute(
            "DELETE FROM bindings WHERE channel=?1 AND external_key=?2",
            params![channel, key],
        )? == 1)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_memory(
        &self,
        id: &str,
        project_id: Option<&str>,
        scope: &str,
        kind: &str,
        tag: &str,
        text: &str,
        now: &str,
    ) -> Result<MemoryRecord> {
        self.state.connection().execute("INSERT INTO memory(id,project_id,scope,kind,tag,text,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?7)", params![id,project_id,scope,kind,tag,text,now])?;
        self.get_memory(id)?.ok_or_else(|| ServiceError::NotFound {
            kind: "memory",
            id: id.into(),
        })
    }
    pub fn get_memory(&self, id: &str) -> Result<Option<MemoryRecord>> {
        Ok(self.state.connection().query_row("SELECT id,project_id,scope,kind,tag,text,created_at,updated_at,deleted_at FROM memory WHERE id=?1",[id],memory_row).optional()?)
    }
    pub fn list_memory(
        &self,
        project_id: Option<&str>,
        scope: Option<&str>,
    ) -> Result<Vec<MemoryRecord>> {
        let mut s=self.state.connection().prepare("SELECT id,project_id,scope,kind,tag,text,created_at,updated_at,deleted_at FROM memory WHERE deleted_at IS NULL AND (?1 IS NULL OR project_id=?1) AND (?2 IS NULL OR scope=?2) ORDER BY created_at")?;
        Ok(s.query_map(params![project_id, scope], memory_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn amend_memory(&self, id: &str, text: &str, now: &str) -> Result<MemoryRecord> {
        if self.state.connection().execute(
            "UPDATE memory SET text=?2,updated_at=?3 WHERE id=?1 AND deleted_at IS NULL",
            params![id, text, now],
        )? == 0
        {
            return Err(ServiceError::NotFound {
                kind: "memory",
                id: id.into(),
            });
        };
        self.get_memory(id)?.ok_or_else(|| ServiceError::NotFound {
            kind: "memory",
            id: id.into(),
        })
    }
    pub fn forget_memory(&self, id: &str, now: &str) -> Result<bool> {
        Ok(self.state.connection().execute(
            "UPDATE memory SET deleted_at=?2,updated_at=?2 WHERE id=?1 AND deleted_at IS NULL",
            params![id, now],
        )? == 1)
    }

    pub fn propose_skill(
        &self,
        id: &str,
        project_id: Option<&str>,
        name: &str,
        description: &str,
        body_digest: Option<&str>,
        now: &str,
    ) -> Result<SkillRecord> {
        self.state.connection().execute("INSERT INTO skills(id,project_id,name,description,body_digest,status,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,'proposed',?6,?6)",params![id,project_id,name,description,body_digest,now])?;
        self.get_skill(id)?.ok_or_else(|| ServiceError::NotFound {
            kind: "skill",
            id: id.into(),
        })
    }
    pub fn get_skill(&self, id: &str) -> Result<Option<SkillRecord>> {
        Ok(self.state.connection().query_row("SELECT id,project_id,name,description,body_digest,status,created_at,updated_at FROM skills WHERE id=?1",[id],skill_row).optional()?)
    }
    pub fn list_skills(
        &self,
        project_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<SkillRecord>> {
        let mut s=self.state.connection().prepare("SELECT id,project_id,name,description,body_digest,status,created_at,updated_at FROM skills WHERE (?1 IS NULL OR project_id=?1) AND (?2 IS NULL OR status=?2) ORDER BY created_at")?;
        Ok(s.query_map(params![project_id, status], skill_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn promote_skill(&self, id: &str, now: &str) -> Result<SkillRecord> {
        if self.state.connection().execute(
            "UPDATE skills SET status='active',updated_at=?2 WHERE id=?1 AND status='proposed'",
            params![id, now],
        )? == 0
        {
            return Err(ServiceError::InvalidTransition {
                from: "missing_or_active".into(),
                to: "active".into(),
            });
        };
        self.get_skill(id)?.ok_or_else(|| ServiceError::NotFound {
            kind: "skill",
            id: id.into(),
        })
    }
    pub fn reject_skill(&self, id: &str, now: &str) -> Result<SkillRecord> {
        if self.state.connection().execute(
            "UPDATE skills SET status='rejected',updated_at=?2 WHERE id=?1 AND status='proposed'",
            params![id, now],
        )? == 0
        {
            return Err(ServiceError::InvalidTransition {
                from: "missing_or_resolved".into(),
                to: "rejected".into(),
            });
        };
        self.get_skill(id)?.ok_or_else(|| ServiceError::NotFound {
            kind: "skill",
            id: id.into(),
        })
    }

    pub fn create_checkpoint(
        &self,
        id: &str,
        session_id: &str,
        manifest: &Value,
        label: &str,
        now: &str,
    ) -> Result<CheckpointRecord> {
        let digest = self.put_blob(&serde_json::to_vec(manifest)?, now)?;
        self.state.connection().execute("INSERT INTO checkpoints(id,session_id,manifest_digest,created_at,label) VALUES(?1,?2,?3,?4,?5)",params![id,session_id,digest,now,label])?;
        self.get_checkpoint(id)?
            .ok_or_else(|| ServiceError::NotFound {
                kind: "checkpoint",
                id: id.into(),
            })
    }
    pub fn get_checkpoint(&self, id: &str) -> Result<Option<CheckpointRecord>> {
        Ok(self.state.connection().query_row("SELECT id,session_id,manifest_digest,created_at,label FROM checkpoints WHERE id=?1",[id],|r|Ok(CheckpointRecord{id:r.get(0)?,session_id:r.get(1)?,manifest_digest:r.get(2)?,created_at:r.get(3)?,label:r.get(4)?})).optional()?)
    }
    pub fn list_checkpoints(&self, session_id: &str) -> Result<Vec<CheckpointRecord>> {
        let mut s=self.state.connection().prepare("SELECT id,session_id,manifest_digest,created_at,label FROM checkpoints WHERE session_id=?1 ORDER BY created_at")?;
        Ok(s.query_map([session_id], |r| {
            Ok(CheckpointRecord {
                id: r.get(0)?,
                session_id: r.get(1)?,
                manifest_digest: r.get(2)?,
                created_at: r.get(3)?,
                label: r.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn index_document(
        &self,
        id: &str,
        project_id: Option<&str>,
        source: &str,
        title: &str,
        body: &str,
        now: &str,
    ) -> Result<()> {
        self.state.connection().execute("INSERT INTO documents(id,project_id,source,title,body,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?6) ON CONFLICT(id) DO UPDATE SET title=excluded.title,body=excluded.body,updated_at=excluded.updated_at",params![id,project_id,source,title,body,now])?;
        Ok(())
    }
    pub fn search_documents(&self, query: &str) -> Result<Vec<(String, String, String)>> {
        let mut s=self.state.connection().prepare("SELECT id,title,source FROM documents WHERE body LIKE '%'||?1||'%' OR title LIKE '%'||?1||'%' ORDER BY updated_at DESC")?;
        Ok(
            s.query_map([query], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_usage(
        &self,
        id: &str,
        run_id: Option<&str>,
        provider: &str,
        model: &str,
        input: i64,
        output: i64,
        cost: Option<i64>,
        now: &str,
    ) -> Result<UsageRecord> {
        self.state.connection().execute("INSERT INTO usage_ledger(id,run_id,provider,model,input_tokens,output_tokens,cost_cents,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![id,run_id,provider,model,input,output,cost,now])?;
        Ok(UsageRecord {
            id: id.into(),
            run_id: run_id.map(str::to_owned),
            provider: provider.into(),
            model: model.into(),
            input_tokens: input,
            output_tokens: output,
            cost_cents: cost,
            created_at: now.into(),
        })
    }
    pub fn set_budget(&self, scope: &str, cap: Option<i64>, now: &str) -> Result<()> {
        self.state.connection().execute("INSERT INTO budgets(scope,cap_cents,updated_at) VALUES(?1,?2,?3) ON CONFLICT(scope) DO UPDATE SET cap_cents=excluded.cap_cents,updated_at=excluded.updated_at",params![scope,cap,now])?;
        Ok(())
    }
    pub fn budget(&self, scope: &str) -> Result<Option<(Option<i64>, i64)>> {
        Ok(self
            .state
            .connection()
            .query_row(
                "SELECT cap_cents,used_cents FROM budgets WHERE scope=?1",
                [scope],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }
    pub fn add_budget_usage(&self, scope: &str, cost: i64, now: &str) -> Result<()> {
        self.state.connection().execute(
            "UPDATE budgets SET used_cents=used_cents+?2,updated_at=?3 WHERE scope=?1",
            params![scope, cost, now],
        )?;
        Ok(())
    }
    pub fn diagnostic(
        &self,
        id: &str,
        kind: &str,
        status: &str,
        detail: &Value,
        now: &str,
    ) -> Result<()> {
        self.state.connection().execute(
            "INSERT INTO diagnostics(id,kind,status,detail_json,created_at) VALUES(?1,?2,?3,?4,?5)",
            params![id, kind, status, serde_json::to_string(detail)?, now],
        )?;
        Ok(())
    }

    pub fn put_blob(&self, bytes: &[u8], size_timestamp: &str) -> Result<String> {
        let digest = self.blobs.put(bytes)?;
        self.state
            .register_blob(&digest, bytes.len() as i64, size_timestamp)?;
        Ok(digest)
    }
    pub fn audit(&self, event: &AuditEvent) -> Result<()> {
        self.audit.append(event).map_err(ServiceError::from)
    }
    pub fn query_audit(&self) -> Result<Vec<AuditEvent>> {
        let path = self.audit.path();
        if !path.exists() {
            return Ok(Vec::new());
        };
        let text = std::fs::read_to_string(path)?;
        text.lines()
            .map(|line| {
                serde_json::from_str(line)
                    .map_err(StorageError::from)
                    .map_err(ServiceError::from)
            })
            .collect()
    }
}

fn parse_json(raw: String) -> rusqlite::Result<Value> {
    serde_json::from_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}
fn task_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRecord> {
    Ok(TaskRecord {
        id: r.get(0)?,
        project_id: r.get(1)?,
        spec: parse_json(r.get(2)?)?,
        status: r.get(3)?,
        updated_at: r.get(4)?,
    })
}
fn memory_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRecord> {
    Ok(MemoryRecord {
        id: r.get(0)?,
        project_id: r.get(1)?,
        scope: r.get(2)?,
        kind: r.get(3)?,
        tag: r.get(4)?,
        text: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
        deleted_at: r.get(8)?,
    })
}
fn skill_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SkillRecord> {
    Ok(SkillRecord {
        id: r.get(0)?,
        project_id: r.get(1)?,
        name: r.get(2)?,
        description: r.get(3)?,
        body_digest: r.get(4)?,
        status: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}
fn approval_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ApprovalRecord> {
    Ok(ApprovalRecord {
        id: r.get(0)?,
        run_id: r.get(1)?,
        status: r.get(2)?,
        request: parse_json(r.get(3)?)?,
        response: r.get::<_, Option<String>>(4)?.map(parse_json).transpose()?,
        created_at: r.get(5)?,
        resolved_at: r.get(6)?,
    })
}
fn inbox_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<InboxRecord> {
    Ok(InboxRecord {
        id: r.get(0)?,
        channel: r.get(1)?,
        external_id: r.get(2)?,
        payload: parse_json(r.get(3)?)?,
        created_at: r.get(4)?,
        acknowledged_at: r.get(5)?,
    })
}
fn delivery_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DeliveryRecord> {
    Ok(DeliveryRecord {
        id: r.get(0)?,
        channel: r.get(1)?,
        payload: parse_json(r.get(2)?)?,
        status: r.get(3)?,
        attempts: r.get(4)?,
        next_attempt_at: r.get(5)?,
        updated_at: r.get(6)?,
        error: r.get(7)?,
    })
}
fn binding_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<BindingRecord> {
    Ok(BindingRecord {
        id: r.get(0)?,
        channel: r.get(1)?,
        external_key: r.get(2)?,
        target: parse_json(r.get(3)?)?,
        created_at: r.get(4)?,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    #[test]
    fn task_approval_and_delivery_lifecycle() -> Result<()> {
        let d = tempdir().unwrap();
        let s = Services::open(d.path())?;
        let t = s.create_task("t", None, &serde_json::json!({"prompt":"x"}), "ready", "1")?;
        assert_eq!(s.get_task("t")?.unwrap(), t);
        s.state.create_project("p", "/tmp/p", "1", "{}")?;
        s.state.create_session("s", "p", "1", "active", "{}")?;
        s.state.create_run("r", "s", "running", "1", 0)?;
        s.create_approval("a", "r", &serde_json::json!({"tool":"write"}), "1")?;
        assert_eq!(s.pending_approvals()?.len(), 1);
        assert_eq!(
            s.resolve_approval("a", true, &serde_json::json!({}), "2")?
                .status,
            "allowed"
        );
        let j = s.enqueue_delivery("d", "test", &serde_json::json!({"x":1}), "1")?;
        assert_eq!(s.claim_delivery("d", "2")?.unwrap().attempts, 1);
        assert_eq!(
            s.mark_delivery_retry("d", "3", "offline", "2")?.status,
            "retry"
        );
        assert_eq!(s.claim_delivery("d", "4")?.unwrap().attempts, 2);
        assert_eq!(s.mark_delivery_delivered("d", "5")?.status, "delivered");
        assert_eq!(j.id, "d");
        Ok(())
    }
    #[test]
    fn inbox_binding_blob_and_audit_survive_restart() -> Result<()> {
        let d = tempdir().unwrap();
        {
            let s = Services::open(d.path())?;
            s.insert_inbox("i", "web", Some("x"), &serde_json::json!({"ok":true}), "1")?;
            s.upsert_binding("b", "web", "x", &serde_json::json!({"session":"s"}), "1")?;
            let digest = s.put_blob(b"hello", "1")?;
            assert!(s.blobs.contains(&digest)?);
            s.audit(&AuditEvent {
                event: "test".into(),
                timestamp: "1".into(),
                actor: "t".into(),
                data: serde_json::json!({}),
            })?;
        }
        let s = Services::open(d.path())?;
        assert!(s.get_inbox("i")?.is_some());
        assert!(s.get_binding("web", "x")?.is_some());
        assert_eq!(s.query_audit()?.len(), 1);
        assert!(s.acknowledge_inbox("i", "2")?);
        Ok(())
    }
}
