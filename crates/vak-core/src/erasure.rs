//! Erasing a conversation (plan M7a-e,
//! docs/design/74-lifecycle-and-data-administration.md §4). Nothing is
//! deleted from a ledger: the conversation's key is destroyed, so its
//! frames and objects stay in place, verify, and can no longer be read.
//! What was derived from it in plaintext (search rows, memory notes,
//! rollups) is removed, and a signed receipt says what was done and what
//! Vak could not reach.

use crate::Core;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use vak_session::objects::{TenantObjects, contributor_scope, conversation_scope};

#[derive(Debug, thiserror::Error)]
pub enum ErasureError {
    #[error("no conversation {0}")]
    NotFound(String),
    #[error("a conversation is erased from the trash; move it there first")]
    NotInTrash,
    #[error("this conversation was already erased")]
    AlreadyErased,
    #[error("this conversation is on hold and cannot be erased")]
    Held,
    #[error("the conversation changed since the preview; look again before erasing")]
    StalePreview,
    #[error("erasure did not finish: {0}")]
    Failed(String),
}

/// Why a conversation was erased.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    /// A person asked.
    Person,
    /// Its time in the trash ended.
    Policy,
}

/// What erasing a conversation would destroy, and the digest a person's
/// confirmation must carry: an erasure acts on what was shown, or not at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Preview {
    pub session_id: String,
    pub digest: String,
    pub trashed_at: Option<DateTime<Utc>>,
    pub held: bool,
    /// This conversation and the workers it delegated to.
    pub conversations: Vec<String>,
    /// Drafts made in it that nobody accepted, saved, starred or shared.
    pub artifacts: Vec<String>,
    pub memory_notes: u64,
    /// Messages and changes already sent outside Vak, which stay sent.
    pub sent_outside: u64,
}

/// What an erasure did. Signed with the tenant's key, so the receipt can
/// be checked with `public_key` alone. It holds ids and counts, never
/// content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub id: String,
    pub at: DateTime<Utc>,
    pub scope: String,
    pub subject: String,
    pub cause: Cause,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<vak_session::ids::PrincipalId>,
    pub conversations: u64,
    pub artifacts: u64,
    pub keys_destroyed: u64,
    /// A digest of every key scope destroyed, in order.
    pub keys_digest: String,
    pub memory_notes_removed: u64,
    pub search_rows_removed: u64,
    pub objects_deleted: u64,
    /// What Vak sent outside itself from this conversation and cannot
    /// take back.
    pub sent_outside: u64,
    /// What this erasure did not reach, in plain words.
    pub not_reached: Vec<String>,
    pub public_key: String,
    pub signature: String,
}

impl Receipt {
    /// The bytes the signature covers: the receipt without its signature.
    fn signed_bytes(&self) -> Vec<u8> {
        let mut unsigned = self.clone();
        unsigned.signature = String::new();
        serde_json::to_vec(&unsigned).unwrap_or_default()
    }

    /// Whether the signature is the public key's over this receipt.
    pub fn verifies(&self) -> bool {
        let unhex = |hex: &str| -> Option<Vec<u8>> {
            (0..hex.len() / 2)
                .map(|i| u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok())
                .collect()
        };
        match (unhex(&self.public_key), unhex(&self.signature)) {
            (Some(key), Some(signature)) => {
                vak_storage::keys::verify(&key, &self.signed_bytes(), &signature)
            }
            _ => false,
        }
    }
}

const NOT_REACHED: &[&str] = &[
    "Copies the AI services received when they answered: they are held by those services under their own terms.",
    "Messages and changes already delivered outside Vakyartha: they stay with whoever received them.",
    "Backups made before this erasure: they still hold the encrypted records until they expire.",
    "Entities and skills an Agent derived from this conversation: they are not examined by this erasure.",
    "What an Agent wrote in other conversations from what it learned here.",
];

const DRAFT_NOT_REACHED: &[&str] = &[
    "The conversation that made this draft: it is kept, with whatever it says about the draft.",
    "Copies the AI services received when they answered: they are held by those services under their own terms.",
    "Copies already downloaded or delivered outside Vakyartha: they stay with whoever received them.",
    "Backups made before this erasure: they still hold the encrypted records until they expire.",
    "Other versions of the same file: only this draft was erased.",
];

/// Everything one erasure will touch, gathered before anything is.
struct Reach {
    conversations: Vec<String>,
    artifacts: Vec<vak_session::ids::ArtifactId>,
    scopes: Vec<String>,
    notes: Vec<(std::path::PathBuf, std::path::PathBuf, String, bool)>,
    sent_outside: u64,
    trashed_at: Option<DateTime<Utc>>,
    erased: bool,
    held: bool,
}

impl Core {
    pub(crate) fn tenant_objects(&self) -> Result<std::sync::Arc<TenantObjects>, ErasureError> {
        TenantObjects::for_tenant(&vak_config::paths::tenant_home_at(
            &self.inner.sessions_home,
            vak_config::paths::LOCAL_TENANT,
        ))
        .map_err(|error| ErasureError::Failed(error.to_string()))
    }

    /// The receipts of every erasure, oldest first.
    pub fn erasure_receipts(&self) -> Vec<Receipt> {
        vak_session::chain::RecordChain::at(self.shared_scope().erasures()).read()
    }

    fn reach(&self, session_id: &str) -> Result<Reach, ErasureError> {
        let failed = |error: String| ErasureError::Failed(error);
        let shared = self.shared_scope();
        let state =
            crate::trash::state(&shared, session_id).map_err(|error| failed(error.to_string()))?;
        let catalog = self.catalog().map_err(|error| failed(error.to_string()))?;
        let _ = catalog.catch_up();
        if state.erased_at.is_none()
            && catalog
                .session_dir(session_id)
                .map_err(|error| failed(error.to_string()))?
                .is_none()
        {
            return Err(ErasureError::NotFound(session_id.to_string()));
        }
        // The conversation and, through them, every worker it caused.
        let mut conversations = vec![session_id.to_string()];
        let mut next = 0;
        while next < conversations.len() {
            let children = catalog
                .session_children(&conversations[next])
                .map_err(|error| failed(error.to_string()))?;
            for child in children {
                let child = child.trim_start_matches("ses_").to_string();
                if !conversations.contains(&child) {
                    conversations.push(child);
                }
            }
            next += 1;
        }
        let ours: BTreeSet<&str> = conversations.iter().map(String::as_str).collect();
        // A draft made here that nobody kept goes with it; anything
        // accepted, saved, starred or shared is a person's own document.
        let grants = crate::grants::Grants::at(&shared);
        let now = Utc::now();
        let mut artifacts = Vec::new();
        for artifact in self.artifacts().list() {
            let shared_out = grants
                .on(&crate::grants::GrantObject::Artifact(artifact.id))
                .ok()
                .is_some_and(|held| {
                    held.iter()
                        .any(|held| held.status(now) == crate::grants::GrantStatus::Active)
                });
            let only_ours = !artifact.versions.is_empty()
                && artifact.versions.iter().all(|version| {
                    !version.promoted
                        && !version.saved
                        && !version.proposed.is_empty()
                        && version
                            .proposed
                            .iter()
                            .all(|proposal| ours.contains(proposal.session.as_str()))
                });
            if only_ours && !artifact.starred && !shared_out {
                artifacts.push(artifact.id);
            }
        }
        let tenant = self.tenant_objects()?;
        let mut scopes = Vec::new();
        for conversation in &conversations {
            scopes.push(conversation_scope(conversation));
            scopes.extend(
                tenant
                    .scopes_with_prefix(&contributor_scope(conversation, ""))
                    .map_err(|error| failed(error.to_string()))?,
            );
        }
        for artifact in &artifacts {
            scopes.push(crate::artifacts::object_scope(artifact));
            scopes.extend(
                tenant
                    .scopes_with_prefix(&contributor_scope(&artifact.to_string(), ""))
                    .map_err(|error| failed(error.to_string()))?,
            );
        }
        let held = scopes.iter().any(|scope| tenant.scope_held(scope));
        // Memory notes written from these conversations, in each Agent's
        // workspace tier for the conversation's folder and its profile tier.
        let mut notes = Vec::new();
        for conversation in &conversations {
            let Ok(Some(dir)) = catalog.session_dir(conversation) else {
                continue;
            };
            let Ok(header) = vak_session::SessionLog::read_header(&dir) else {
                continue;
            };
            let agent = header
                .agent
                .as_ref()
                .map_or("vak", |agent| agent.id.as_str());
            let home = shared.agents_dir().join(agent);
            for (profile, listed) in [
                (false, crate::memory::list_notes(&home, &header.cwd)),
                (true, crate::memory::list_profile_notes(&home)),
            ] {
                for note in listed {
                    if ours.contains(note.session_id.as_str()) {
                        notes.push((home.clone(), header.cwd.clone(), note.id, profile));
                    }
                }
            }
        }
        notes.sort();
        notes.dedup();
        let sent_outside = self
            .effects()
            .list()
            .unwrap_or_default()
            .iter()
            .filter(|effect| effect.status == vak_session::effects::EffectStatus::Sent)
            .filter(|effect| {
                effect
                    .trace
                    .as_ref()
                    .and_then(|trace| trace.session_id())
                    .is_some_and(|session| ours.contains(session.as_str()))
            })
            .count() as u64;
        Ok(Reach {
            conversations,
            artifacts,
            scopes,
            notes,
            sent_outside,
            trashed_at: state.trashed_at,
            erased: state.erased_at.is_some(),
            held,
        })
    }

    fn preview_of(session_id: &str, reach: &Reach) -> Preview {
        let mut hash = Sha256::new();
        for part in reach
            .scopes
            .iter()
            .cloned()
            .chain(reach.notes.iter().map(|note| note.2.clone()))
            .chain([
                format!("{:?}", reach.trashed_at),
                reach.sent_outside.to_string(),
            ])
        {
            hash.update(part.as_bytes());
            hash.update([0]);
        }
        Preview {
            session_id: session_id.to_string(),
            digest: hash.finalize().iter().map(|b| format!("{b:02x}")).collect(),
            trashed_at: reach.trashed_at,
            held: reach.held,
            conversations: reach.conversations.clone(),
            artifacts: reach.artifacts.iter().map(ToString::to_string).collect(),
            memory_notes: reach.notes.len() as u64,
            sent_outside: reach.sent_outside,
        }
    }

    /// What erasing `session_id` would destroy. Reads only.
    pub fn erasure_preview(&self, session_id: &str) -> Result<Preview, ErasureError> {
        let reach = self.reach(session_id)?;
        if reach.erased {
            return Err(ErasureError::AlreadyErased);
        }
        Ok(Self::preview_of(session_id, &reach))
    }

    /// What a person types to confirm erasing `session_id`: the start of
    /// the first thing they said in it, which is the title the lists show,
    /// or the start of its id when it says nothing readable.
    pub fn erasure_confirmation(&self, session_id: &str) -> String {
        let mut title = String::new();
        if let Some(ledger) = self
            .catalog()
            .ok()
            .and_then(|catalog| catalog.session_dir(session_id).ok().flatten())
        {
            vak_session::SessionLog::scan(&ledger, |entry| {
                if let Some(vak_session::EntryPayload::Message(record)) =
                    entry.map(|entry| &entry.payload)
                    && record.message.role == vak_llm::Role::User
                    && record.control_kind().is_none()
                {
                    let text = record.message.text_content();
                    let line = text.trim().lines().next().unwrap_or_default().trim();
                    title = line.chars().take(40).collect::<String>().trim().to_string();
                }
                title.is_empty()
            });
        }
        if title.is_empty() {
            title = session_id.chars().take(8).collect();
        }
        title
    }

    /// Erases `session_id`. A person's erasure carries the digest of the
    /// preview they confirmed and is refused when what it would destroy
    /// has changed since; the reconciler's, at the end of the trash
    /// window, carries none. Refused while anything in reach is on hold,
    /// and for a conversation that is not in the trash.
    pub fn erase_conversation(
        &self,
        session_id: &str,
        digest: Option<&str>,
        cause: Cause,
        actor: Option<vak_session::ids::PrincipalId>,
    ) -> Result<Receipt, ErasureError> {
        let failed = |error: String| ErasureError::Failed(error);
        vak_session::fence::check().map_err(|error| failed(error.to_string()))?;
        let reach = self.reach(session_id)?;
        if reach.erased {
            return Err(ErasureError::AlreadyErased);
        }
        if reach.trashed_at.is_none() {
            return Err(ErasureError::NotInTrash);
        }
        if reach.held {
            return Err(ErasureError::Held);
        }
        if digest.is_some_and(|digest| digest != Self::preview_of(session_id, &reach).digest) {
            return Err(ErasureError::StalePreview);
        }
        let tenant = self.tenant_objects()?;
        // Derived plaintext first, while the conversation can still be
        // named: memory notes, then the search rows.
        let mut notes_removed = 0;
        for (home, cwd, note, profile) in &reach.notes {
            let forgotten = if *profile {
                crate::memory::forget_profile_note(home, note)
            } else {
                crate::memory::forget_workspace_note(home, cwd, note)
            };
            notes_removed += u64::from(forgotten.is_ok());
        }
        let catalog = self.catalog().map_err(|error| failed(error.to_string()))?;
        let artifact_nodes: Vec<String> = reach.artifacts.iter().map(ToString::to_string).collect();
        let search_rows_removed = catalog
            .erase(&reach.conversations, &artifact_nodes)
            .map_err(|error| failed(error.to_string()))?;
        // The keys. After this nothing of the conversation can be read.
        for scope in &reach.scopes {
            tenant
                .destroy_scope_key(scope)
                .map_err(|error| failed(error.to_string()))?;
        }
        let now = Utc::now();
        let shared = self.shared_scope();
        crate::trash::mark_erased(&shared, &reach.conversations)
            .map_err(|error| failed(error.to_string()))?;
        // Rollups that folded its rows are folded again without them.
        let _ = vak_session::documents::forget(&shared.artifacts_rollup());
        for (_, home) in vak_session_agent_homes(&shared) {
            let scope = vak_config::scope::AgentScope::new(home);
            let _ = vak_session::documents::forget(&scope.commitments_rollup());
        }
        // Objects whose last grant went with the keys.
        let objects_deleted = {
            use vak_session::objects::Objects;
            tenant.collect().unwrap_or(0) as u64
        };
        let mut keys = Sha256::new();
        for scope in &reach.scopes {
            keys.update(scope.as_bytes());
            keys.update([0]);
        }
        let receipt = Receipt {
            id: format!("ers_{}", uuid::Uuid::now_v7()),
            at: now,
            scope: "conversation".into(),
            subject: session_id.to_string(),
            cause,
            actor,
            conversations: reach.conversations.len() as u64,
            artifacts: reach.artifacts.len() as u64,
            keys_destroyed: reach.scopes.len() as u64,
            keys_digest: keys.finalize().iter().map(|b| format!("{b:02x}")).collect(),
            memory_notes_removed: notes_removed,
            search_rows_removed,
            objects_deleted,
            sent_outside: reach.sent_outside,
            not_reached: NOT_REACHED.iter().map(ToString::to_string).collect(),
            public_key: String::new(),
            signature: String::new(),
        };
        let receipt = self.sign_and_record(&tenant, receipt)?;
        tracing::info!(
            kind = "erasure",
            outcome = "completed",
            count = receipt.keys_destroyed,
            "a conversation was erased"
        );
        Ok(receipt)
    }

    /// Signs a receipt with the tenant's key and appends it to the
    /// erasures chain. The public key is part of what is signed.
    fn sign_and_record(
        &self,
        tenant: &TenantObjects,
        mut receipt: Receipt,
    ) -> Result<Receipt, ErasureError> {
        let failed = |error: String| ErasureError::Failed(error);
        let (_, public_key) = tenant
            .sign(b"")
            .map_err(|error| failed(error.to_string()))?;
        receipt.public_key = public_key;
        let (signature, _) = tenant
            .sign(&receipt.signed_bytes())
            .map_err(|error| failed(error.to_string()))?;
        receipt.signature = signature;
        vak_session::chain::RecordChain::at(self.shared_scope().erasures())
            .append(&receipt)
            .map_err(|error| failed(error.to_string()))?;
        Ok(receipt)
    }

    /// Erases a draft version from the trash: its bytes are released and
    /// deleted once nothing else holds them. When that leaves the artifact
    /// with no version, and nobody starred or shared it, the artifact goes
    /// whole: its keys are destroyed and its search rows removed. Refused
    /// while the artifact's key is on hold.
    pub fn erase_draft(
        &self,
        artifact: &str,
        version: &str,
        cause: Cause,
        actor: Option<vak_session::ids::PrincipalId>,
    ) -> Result<Receipt, ErasureError> {
        let failed = |error: String| ErasureError::Failed(error);
        vak_session::fence::check().map_err(|error| failed(error.to_string()))?;
        let subject = format!("{artifact}/{version}");
        let missing = || ErasureError::Failed("no such draft".into());
        let artifact_id = vak_session::ids::ArtifactId::parse(artifact).map_err(|_| missing())?;
        let version_id = vak_session::ids::VersionId::parse(version).map_err(|_| missing())?;
        let artifacts = self.artifacts();
        let current = artifacts.get(artifact).ok_or_else(missing)?;
        let found = current
            .versions
            .iter()
            .find(|known| known.id == version_id)
            .ok_or_else(missing)?;
        if found.erased {
            return Err(ErasureError::AlreadyErased);
        }
        if found.trashed_at.is_none() {
            return Err(ErasureError::NotInTrash);
        }
        let tenant = self.tenant_objects()?;
        let object_scope = crate::artifacts::object_scope(&artifact_id);
        if tenant.scope_held(&object_scope) {
            return Err(ErasureError::Held);
        }
        artifacts
            .erase_version(artifact_id, version_id, actor)
            .map_err(|error| failed(error.to_string()))?;
        let shared = self.shared_scope();
        let shared_out = crate::grants::Grants::at(&shared)
            .on(&crate::grants::GrantObject::Artifact(artifact_id))
            .map_err(|error| failed(error.to_string()))?
            .iter()
            .any(|held| held.status(Utc::now()) == crate::grants::GrantStatus::Active);
        let whole = !current.starred
            && !shared_out
            && current
                .versions
                .iter()
                .all(|known| known.erased || known.id == version_id);
        let mut scopes = Vec::new();
        let mut search_rows_removed = 0;
        if whole {
            search_rows_removed = self
                .catalog()
                .map_err(|error| failed(error.to_string()))?
                .erase(&[], &[artifact.to_string()])
                .map_err(|error| failed(error.to_string()))?;
            scopes.push(object_scope);
            scopes.extend(
                tenant
                    .scopes_with_prefix(&contributor_scope(artifact, ""))
                    .map_err(|error| failed(error.to_string()))?,
            );
            for scope in &scopes {
                tenant
                    .destroy_scope_key(scope)
                    .map_err(|error| failed(error.to_string()))?;
            }
            let _ = vak_session::documents::forget(&shared.artifacts_rollup());
        }
        let objects_deleted = {
            use vak_session::objects::Objects;
            tenant.collect().unwrap_or(0) as u64
        };
        let mut keys = Sha256::new();
        for scope in &scopes {
            keys.update(scope.as_bytes());
            keys.update([0]);
        }
        let receipt = Receipt {
            id: format!("ers_{}", uuid::Uuid::now_v7()),
            at: Utc::now(),
            scope: "draft".into(),
            subject,
            cause,
            actor,
            conversations: 0,
            artifacts: u64::from(whole),
            keys_destroyed: scopes.len() as u64,
            keys_digest: keys.finalize().iter().map(|b| format!("{b:02x}")).collect(),
            memory_notes_removed: 0,
            search_rows_removed,
            objects_deleted,
            sent_outside: 0,
            not_reached: DRAFT_NOT_REACHED.iter().map(ToString::to_string).collect(),
            public_key: String::new(),
            signature: String::new(),
        };
        let receipt = self.sign_and_record(&tenant, receipt)?;
        tracing::info!(
            kind = "erasure",
            outcome = "completed",
            count = receipt.objects_deleted,
            "a draft was erased"
        );
        Ok(receipt)
    }

    /// Puts a conversation on hold or releases it: while held, neither a
    /// person nor the end of the trash window erases it (doc 74 §3.2).
    pub fn hold_conversation(&self, session_id: &str, held: bool) -> Result<(), ErasureError> {
        let tenant = self.tenant_objects()?;
        let scope = conversation_scope(session_id);
        if held {
            tenant.hold_scope(&scope)
        } else {
            tenant.release_scope(&scope)
        }
        .map_err(|error| ErasureError::Failed(error.to_string()))
    }

    pub fn conversation_held(&self, session_id: &str) -> bool {
        self.tenant_objects()
            .is_ok_and(|tenant| tenant.scope_held(&conversation_scope(session_id)))
    }
}

/// Every Agent home under the data home, by Agent id.
fn vak_session_agent_homes(
    shared: &vak_config::scope::SharedScope,
) -> Vec<(String, std::path::PathBuf)> {
    std::fs::read_dir(shared.agents_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| Some((entry.file_name().into_string().ok()?, entry.path())))
        .collect()
}
