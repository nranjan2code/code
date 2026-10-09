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
    #[error("this account is still connected; disconnect it before erasing what it returned")]
    StillConnected,
    #[error("this Agent is still in use; revoke or archive it before erasing what it holds")]
    AgentInUse,
    #[error("erasure did not finish: {0}")]
    Failed(String),
}

/// One erasure at a time in a process. An erasure reads the catalog to
/// find what it reaches, and some end by rebuilding it; one that read
/// during another's rebuild would reach less than it should.
static ERASING: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    ERASING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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

const GUEST_NOT_REACHED: &[&str] = &[
    "What the Agent and the other people wrote in reply: it is the conversation's, and stays.",
    "Summaries and memory notes an Agent wrote from what this person said: they are not examined by this erasure.",
    "Copies the AI services received when they answered: they are held by those services under their own terms.",
    "Backups made before this erasure: they still hold the encrypted records until they expire.",
    "This person's access: erasing what they wrote does not end an invitation or a share.",
];

const ACCOUNT_NOT_REACHED: &[&str] = &[
    "What an Agent wrote from this account's data (an answer, a card, a summary, a later turn that repeats it): it is each conversation's own content and stays until that conversation is erased.",
    "Messages and calendar changes already sent through the account: they stay with the provider and whoever received them.",
    "Copies the AI services received when they answered: they are held by those services under their own terms.",
    "Backups made before this erasure: they still hold the encrypted records until they expire.",
    "The account itself and what the provider keeps: this erases only what Vakyartha stored.",
];

const AGENT_NOT_REACHED: &[&str] = &[
    "Files the Agent wrote that someone accepted, saved, starred or shared: they are that person's documents and stay in the Library.",
    "Files it wrote into a project folder of yours: Vakyartha never deletes from a folder you own.",
    "Its run, cost and delivery records: they hold ids, times and numbers, with nothing readable of what was said.",
    "Messages and changes it already sent outside Vakyartha: they stay with whoever received them.",
    "Copies the AI services received when they answered: they are held by those services under their own terms.",
    "Backups made before this erasure: they still hold the encrypted records until they expire.",
];

const PERSON_NOT_REACHED: &[&str] = &[
    "Conversations this person shared with other people (a group chat): what they wrote there is part of that conversation and cannot be separated from it. Erase the conversation to remove it.",
    "What an Agent wrote elsewhere from what it learned from them, and memory notes it kept: they are not examined by this erasure.",
    "Messages already delivered to them or to anyone else: they stay with whoever received them.",
    "Copies the AI services received when they answered: they are held by those services under their own terms.",
    "Backups made before this erasure: they still hold the encrypted records until they expire.",
];

/// What erasing one person would destroy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersonPreview {
    pub digest: String,
    pub held: bool,
    /// Conversations that are theirs alone: erased whole.
    pub conversations: Vec<String>,
    /// Conversations they share with other people: not touched.
    pub shared_conversations: u64,
    /// Which of the identities asked about have a conversation of their
    /// own, so the caller can drop the chat each belongs to.
    pub alone: Vec<String>,
}

const PROJECT_NOT_REACHED: &[&str] = &[
    "The project's own folder, its files and its settings: Vakyartha never deletes from a folder you own.",
    "Run, cost and delivery records: they hold ids, times and numbers, with nothing readable of what was said.",
    "Messages and changes already sent outside Vakyartha: they stay with whoever received them.",
    "Copies the AI services received when they answered: they are held by those services under their own terms.",
    "Backups made before this erasure: they still hold the encrypted records until they expire.",
];

const INSTALL_NOT_REACHED: &[&str] = &[
    "Your project folders and your Vakyartha folder, with their files and settings: Vakyartha never deletes from a folder you own.",
    "Messages and changes already sent outside Vakyartha: they stay with whoever received them.",
    "Copies the AI services received when they answered: they are held by those services under their own terms.",
    "Backups made before this erasure: one that holds your keys can still be restored.",
];

/// The words a person types to erase the whole install.
pub const INSTALL_CONFIRMATION: &str = "erase everything";

/// What erasing the whole install would destroy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallPreview {
    pub digest: String,
    /// How many things are on hold; anything on hold refuses the erasure.
    pub held: u64,
    pub conversations: u64,
    pub artifacts: u64,
    pub keys: u64,
}

/// What erasing an Agent's data, or a project's, would destroy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentPreview {
    pub agent: String,
    pub digest: String,
    pub held: bool,
    pub conversations: u64,
    /// Its memory notes, entities and other Documents.
    pub documents: u64,
    /// The files that go: an Agent's drafts nobody kept, or every copy
    /// kept for a project.
    pub artifacts: u64,
    pub automations: u64,
    /// Files in the workspace Vakyartha keeps for it.
    pub workspace_files: u64,
}

/// Everything an Agent's erasure will touch.
struct AgentReach {
    conversations: Vec<String>,
    scopes: Vec<String>,
    artifacts: Vec<vak_session::ids::ArtifactId>,
    documents: Vec<std::path::PathBuf>,
    triggers: Vec<String>,
    workspaces: Vec<std::path::PathBuf>,
    workspace_files: u64,
    held: bool,
}

fn files_under(dir: &std::path::Path) -> u64 {
    let mut count = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).into_iter().flatten().flatten() {
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(entry.path()),
                Ok(_) => count += 1,
                Err(_) => {}
            }
        }
    }
    count
}

/// Something on hold: nothing erases it, by a person or by a rule, until
/// the hold is released.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hold {
    /// `conversation` or `artifact`; `other` for a key held by hand.
    pub kind: &'static str,
    pub id: String,
    /// What the owner knows it by: a conversation's title, a file's name.
    pub name: Option<String>,
}

/// What erasing one person's contributions to a conversation would
/// destroy, and the digest a confirmation must carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuestPreview {
    pub session_id: String,
    pub principal: String,
    pub digest: String,
    pub held: bool,
    /// Whether they wrote in the conversation itself.
    pub messages: bool,
    /// The conversation's artifacts they commented on.
    pub artifacts: Vec<String>,
}

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

    /// The receipts of every erasure, oldest first: what an erasure of
    /// the whole install left, then this install's own.
    pub fn erasure_receipts(&self) -> Vec<Receipt> {
        let mut receipts = self.install_receipts();
        receipts.extend(vak_session::chain::RecordChain::at(self.shared_scope().erasures()).read());
        receipts
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
        let _turn = one_at_a_time();
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
        let _turn = one_at_a_time();
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

    fn agent_reach(&self, agent: &str) -> Result<AgentReach, ErasureError> {
        let failed = |error: String| ErasureError::Failed(error);
        let plain = !agent.is_empty()
            && agent
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !plain || agent == "vak" {
            return Err(ErasureError::NotFound(agent.to_string()));
        }
        let shared = self.shared_scope();
        let home = shared.agents_dir().join(agent);
        let saved = crate::agent_definitions::effective(self)
            .unwrap_or_default()
            .into_iter()
            .find(|saved| saved.id == agent);
        if saved.is_none() && !home.is_dir() {
            return Err(ErasureError::NotFound(agent.to_string()));
        }
        if saved.is_some_and(|saved| {
            matches!(
                saved.lifecycle,
                crate::agent_definitions::AgentLifecycle::Active
                    | crate::agent_definitions::AgentLifecycle::Paused
            )
        }) {
            return Err(ErasureError::AgentInUse);
        }
        let tenant = self.tenant_objects()?;
        let scope = vak_config::scope::AgentScope::new(&home);
        // Its conversations: every ledger in its home.
        let mut conversations = Vec::new();
        for space in std::fs::read_dir(scope.sessions_root())
            .into_iter()
            .flatten()
            .flatten()
        {
            for ledger in std::fs::read_dir(space.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                if let Some(id) = vak_config::scope::ledger_session_id(&ledger.path()) {
                    conversations.push(id);
                }
            }
        }
        conversations.sort();
        // One already erased has nothing left to erase.
        conversations.retain(|id| {
            crate::trash::state(&shared, id).map_or(true, |state| state.erased_at.is_none())
        });
        let mut scopes = Vec::new();
        for conversation in &conversations {
            for prefix in [
                conversation_scope(conversation),
                contributor_scope(conversation, ""),
            ] {
                scopes.extend(
                    tenant
                        .scopes_with_prefix(&prefix)
                        .map_err(|error| failed(error.to_string()))?
                        .into_iter()
                        // A prefix also matches a longer conversation id.
                        .filter(|found| {
                            found == &conversation_scope(conversation)
                                || found.starts_with(&contributor_scope(conversation, ""))
                        }),
                );
            }
        }
        // Drafts it made that nobody kept. What a person accepted, saved,
        // starred or shared is theirs.
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
            let unkept = artifact
                .versions
                .iter()
                .all(|version| !version.promoted && !version.saved);
            if artifact.agent == agent && unkept && !artifact.starred && !shared_out {
                scopes.push(crate::artifacts::object_scope(&artifact.id));
                scopes.extend(
                    tenant
                        .scopes_with_prefix(&contributor_scope(&artifact.id.to_string(), ""))
                        .map_err(|error| failed(error.to_string()))?,
                );
                artifacts.push(artifact.id);
            }
        }
        scopes.sort();
        scopes.dedup();
        let held = scopes.iter().any(|scope| tenant.scope_held(scope));
        let documents = vak_session::documents::under(&home);
        let triggers = crate::triggers::list(&shared)
            .unwrap_or_default()
            .into_iter()
            .filter(|trigger| trigger.agent == agent)
            .map(|trigger| trigger.id.to_string())
            .collect();
        // The workspace Vakyartha keeps for it in each project, in the data
        // home. Never a folder the owner made.
        let mut workspaces = Vec::new();
        let tenant_home = vak_config::paths::tenant_home_at(
            &shared.clone().into_root(),
            vak_config::paths::LOCAL_TENANT,
        );
        for space in std::fs::read_dir(tenant_home.join("workspaces"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let workspace = space.path().join(agent);
            if workspace.is_dir() {
                workspaces.push(workspace);
            }
        }
        workspaces.sort();
        let workspace_files = workspaces.iter().map(|dir| files_under(dir)).sum();
        Ok(AgentReach {
            conversations,
            scopes,
            artifacts,
            documents,
            triggers,
            workspaces,
            workspace_files,
            held,
        })
    }

    fn agent_preview_of(agent: &str, reach: &AgentReach) -> AgentPreview {
        let mut digest = Sha256::new();
        for part in reach
            .conversations
            .iter()
            .chain(&reach.scopes)
            .chain(&reach.triggers)
        {
            digest.update(part.as_bytes());
            digest.update([0]);
        }
        for artifact in &reach.artifacts {
            digest.update(artifact.to_string().as_bytes());
            digest.update([0]);
        }
        digest.update(reach.documents.len().to_le_bytes());
        digest.update(reach.workspace_files.to_le_bytes());
        AgentPreview {
            agent: agent.to_string(),
            digest: digest
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            held: reach.held,
            conversations: reach.conversations.len() as u64,
            documents: reach.documents.len() as u64,
            artifacts: reach.artifacts.len() as u64,
            automations: reach.triggers.len() as u64,
            workspace_files: reach.workspace_files,
        }
    }

    /// What erasing everything `agent` holds would destroy.
    pub fn agent_erasure_preview(&self, agent: &str) -> Result<AgentPreview, ErasureError> {
        let _turn = one_at_a_time();
        Ok(Self::agent_preview_of(agent, &self.agent_reach(agent)?))
    }

    /// Erases everything a revoked or archived Agent holds: every
    /// conversation in its home (their keys are destroyed, so nothing of
    /// them reads again), its memory and other Documents, the drafts it
    /// made that nobody kept, its automations, and the workspace
    /// Vakyartha keeps for it. What a person kept of its work stays.
    /// Refused while it is active or paused, while anything in reach is on
    /// hold, and when a confirmation's digest no longer matches.
    pub fn erase_agent(
        &self,
        agent: &str,
        digest: Option<&str>,
        cause: Cause,
        actor: Option<vak_session::ids::PrincipalId>,
    ) -> Result<Receipt, ErasureError> {
        let _turn = one_at_a_time();
        let failed = |error: String| ErasureError::Failed(error);
        vak_session::fence::check().map_err(|error| failed(error.to_string()))?;
        let reach = self.agent_reach(agent)?;
        if reach.held {
            return Err(ErasureError::Held);
        }
        if digest.is_some_and(|digest| digest != Self::agent_preview_of(agent, &reach).digest) {
            return Err(ErasureError::StalePreview);
        }
        self.erase_reach(&reach, "agent", agent, AGENT_NOT_REACHED, cause, actor)
    }

    /// The part an Agent's and a project's erasure share, once the reach
    /// is settled: the Documents, automations and folders in the data
    /// home, then the keys, search and the receipt.
    fn erase_reach(
        &self,
        reach: &AgentReach,
        scope: &str,
        subject: &str,
        not_reached: &[&str],
        cause: Cause,
        actor: Option<vak_session::ids::PrincipalId>,
    ) -> Result<Receipt, ErasureError> {
        let failed = |error: String| ErasureError::Failed(error);
        let tenant = self.tenant_objects()?;
        let shared = self.shared_scope();
        let mut documents = 0;
        for document in &reach.documents {
            documents += u64::from(vak_session::documents::forget(document).unwrap_or(false));
        }
        for trigger in &reach.triggers {
            let _ = crate::triggers::delete(&shared, trigger);
        }
        for workspace in &reach.workspaces {
            std::fs::remove_dir_all(workspace).map_err(|error| failed(error.to_string()))?;
        }
        crate::trash::mark_erased(&shared, &reach.conversations)
            .map_err(|error| failed(error.to_string()))?;
        for (_, home) in vak_session_agent_homes(&shared) {
            let agent = vak_config::scope::AgentScope::new(home);
            let _ = vak_session::documents::forget(&agent.commitments_rollup());
        }
        let mut receipt = Receipt {
            id: format!("ers_{}", uuid::Uuid::now_v7()),
            at: Utc::now(),
            scope: scope.to_string(),
            subject: subject.to_string(),
            cause,
            actor,
            conversations: reach.conversations.len() as u64,
            artifacts: reach.artifacts.len() as u64,
            keys_destroyed: 0,
            keys_digest: String::new(),
            memory_notes_removed: documents,
            search_rows_removed: 0,
            objects_deleted: 0,
            sent_outside: 0,
            not_reached: not_reached.iter().map(ToString::to_string).collect(),
            public_key: String::new(),
            signature: String::new(),
        };
        receipt.sent_outside = self
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
                    .is_some_and(|session| reach.conversations.contains(&session))
            })
            .count() as u64;
        self.destroy_and_record(&tenant, &reach.scopes, receipt)
    }

    /// Sorts the conversations in which any of `principals` caused a run
    /// into theirs alone and shared with other people, with the keys an
    /// erasure of theirs would destroy.
    fn person_reach(
        &self,
        principals: &[String],
    ) -> Result<(PersonPreview, Vec<String>), ErasureError> {
        let failed = |error: String| ErasureError::Failed(error);
        let shared = self.shared_scope();
        let tenant = self.tenant_objects()?;
        let catalog = self.catalog().map_err(|error| failed(error.to_string()))?;
        let _ = catalog.catch_up();
        // Whoever else may cause a run in a person's own conversation
        // without being another person: the owner, the system, an Agent.
        let mut neutral: BTreeSet<String> = [
            vak_session::trace::local::local_owner().to_string(),
            vak_session::trace::local::system_principal().to_string(),
            vak_session::trace::local::agent_principal("vak").to_string(),
        ]
        .into();
        for (agent, _) in vak_session_agent_homes(&shared) {
            neutral.insert(vak_session::trace::local::agent_principal(&agent).to_string());
        }
        let mut conversations = Vec::new();
        let mut alone = Vec::new();
        let mut shared_with_others: BTreeSet<String> = BTreeSet::new();
        for principal in principals {
            for session in catalog
                .sessions_of_actor(principal)
                .map_err(|error| failed(error.to_string()))?
            {
                let erased = crate::trash::state(&shared, &session)
                    .is_ok_and(|state| state.erased_at.is_some());
                if erased {
                    continue;
                }
                let others = catalog
                    .run_actors(&session)
                    .map_err(|error| failed(error.to_string()))?
                    .into_iter()
                    .any(|actor| !principals.contains(&actor) && !neutral.contains(&actor));
                if others {
                    shared_with_others.insert(session);
                } else {
                    if !alone.contains(principal) {
                        alone.push(principal.clone());
                    }
                    if !conversations.contains(&session) {
                        conversations.push(session);
                    }
                }
            }
        }
        conversations.sort();
        let mut scopes = Vec::new();
        for conversation in &conversations {
            for prefix in [
                conversation_scope(conversation),
                contributor_scope(conversation, ""),
            ] {
                scopes.extend(
                    tenant
                        .scopes_with_prefix(&prefix)
                        .map_err(|error| failed(error.to_string()))?
                        .into_iter()
                        .filter(|found| {
                            found == &conversation_scope(conversation)
                                || found.starts_with(&contributor_scope(conversation, ""))
                        }),
                );
            }
        }
        // What they wrote as a guest anywhere is under a key of their own.
        for scope in tenant
            .scopes_with_prefix("contributor:")
            .map_err(|error| failed(error.to_string()))?
        {
            let theirs = vak_session::objects::contributor_of(&scope)
                .is_some_and(|who| principals.iter().any(|principal| principal == who));
            if theirs {
                scopes.push(scope);
            }
        }
        scopes.sort();
        scopes.dedup();
        let mut digest = Sha256::new();
        for part in conversations.iter().chain(&scopes) {
            digest.update(part.as_bytes());
            digest.update([0]);
        }
        let held = scopes.iter().any(|scope| tenant.scope_held(scope));
        Ok((
            PersonPreview {
                digest: digest
                    .finalize()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
                held,
                conversations,
                shared_conversations: shared_with_others.len() as u64,
                alone,
            },
            scopes,
        ))
    }

    /// What erasing the person behind `principals` (one identity for each
    /// chat they wrote in) would destroy.
    pub fn person_erasure_preview(
        &self,
        principals: &[String],
    ) -> Result<PersonPreview, ErasureError> {
        let _turn = one_at_a_time();
        Ok(self.person_reach(principals)?.0)
    }

    /// Erases one person: every conversation that is theirs alone, whole,
    /// and whatever they wrote as a guest in someone else's. A
    /// conversation they share with other people is not touched, and the
    /// receipt says how many there are. `fingerprint` names them in the
    /// receipt without saying who they are. Refused when they have nothing
    /// that can be erased, while anything in reach is on hold, and when a
    /// confirmation's digest no longer matches.
    pub fn erase_person(
        &self,
        fingerprint: &str,
        principals: &[String],
        digest: Option<&str>,
        cause: Cause,
        actor: Option<vak_session::ids::PrincipalId>,
    ) -> Result<Receipt, ErasureError> {
        let _turn = one_at_a_time();
        let failed = |error: String| ErasureError::Failed(error);
        vak_session::fence::check().map_err(|error| failed(error.to_string()))?;
        let (preview, scopes) = self.person_reach(principals)?;
        if preview.conversations.is_empty() && scopes.is_empty() {
            return Err(ErasureError::NotFound(fingerprint.to_string()));
        }
        if preview.held {
            return Err(ErasureError::Held);
        }
        if digest.is_some_and(|digest| digest != preview.digest) {
            return Err(ErasureError::StalePreview);
        }
        let tenant = self.tenant_objects()?;
        let shared = self.shared_scope();
        crate::trash::mark_erased(&shared, &preview.conversations)
            .map_err(|error| failed(error.to_string()))?;
        for (_, home) in vak_session_agent_homes(&shared) {
            let agent = vak_config::scope::AgentScope::new(home);
            let _ = vak_session::documents::forget(&agent.commitments_rollup());
        }
        let mut not_reached: Vec<String> =
            PERSON_NOT_REACHED.iter().map(ToString::to_string).collect();
        if preview.shared_conversations > 0 {
            not_reached.insert(
                0,
                format!(
                    "{} conversation(s) they share with other people were not touched.",
                    preview.shared_conversations
                ),
            );
        }
        let receipt = Receipt {
            id: format!("ers_{}", uuid::Uuid::now_v7()),
            at: Utc::now(),
            scope: "person".into(),
            subject: fingerprint.to_string(),
            cause,
            actor,
            conversations: preview.conversations.len() as u64,
            artifacts: 0,
            keys_destroyed: 0,
            keys_digest: String::new(),
            memory_notes_removed: 0,
            search_rows_removed: 0,
            objects_deleted: 0,
            sent_outside: 0,
            not_reached,
            public_key: String::new(),
            signature: String::new(),
        };
        self.destroy_and_record(&tenant, &scopes, receipt)
    }

    fn project_reach(&self, space: &str) -> Result<AgentReach, ErasureError> {
        let failed = |error: String| ErasureError::Failed(error);
        let known = vak_config::spaces::all()
            .into_iter()
            .any(|record| record.id == space);
        if !known || !space.starts_with("spc_") {
            return Err(ErasureError::NotFound(space.to_string()));
        }
        let shared = self.shared_scope();
        let tenant = self.tenant_objects()?;
        // Every Agent's conversations in this project, and every Agent's
        // Documents kept for it.
        let mut conversations = Vec::new();
        let mut documents = Vec::new();
        for (_, home) in vak_session_agent_homes(&shared) {
            let agent = vak_config::scope::AgentScope::new(&home);
            for ledger in std::fs::read_dir(agent.sessions_root().join(space))
                .into_iter()
                .flatten()
                .flatten()
            {
                if let Some(id) = vak_config::scope::ledger_session_id(&ledger.path()) {
                    conversations.push(id);
                }
            }
            documents.extend(
                vak_session::documents::under(&home)
                    .into_iter()
                    .filter(|path| path.components().any(|part| part.as_os_str() == space)),
            );
        }
        conversations.sort();
        // One already erased has nothing left to erase.
        conversations.retain(|id| {
            crate::trash::state(&shared, id).map_or(true, |state| state.erased_at.is_none())
        });
        let mut scopes = Vec::new();
        for conversation in &conversations {
            for prefix in [
                conversation_scope(conversation),
                contributor_scope(conversation, ""),
            ] {
                scopes.extend(
                    tenant
                        .scopes_with_prefix(&prefix)
                        .map_err(|error| failed(error.to_string()))?
                        .into_iter()
                        .filter(|found| {
                            found == &conversation_scope(conversation)
                                || found.starts_with(&contributor_scope(conversation, ""))
                        }),
                );
            }
        }
        // Every file Vakyartha kept a copy of for this project, kept or
        // not: the project's own folder still has what was accepted.
        let mut artifacts = Vec::new();
        for artifact in self.artifacts().list() {
            if artifact.space == space {
                scopes.push(crate::artifacts::object_scope(&artifact.id));
                scopes.extend(
                    tenant
                        .scopes_with_prefix(&contributor_scope(&artifact.id.to_string(), ""))
                        .map_err(|error| failed(error.to_string()))?,
                );
                artifacts.push(artifact.id);
            }
        }
        scopes.sort();
        scopes.dedup();
        let held = scopes.iter().any(|scope| tenant.scope_held(scope));
        let triggers = crate::triggers::list(&shared)
            .unwrap_or_default()
            .into_iter()
            .filter(|trigger| trigger.space == space)
            .map(|trigger| trigger.id.to_string())
            .collect();
        // What is kept for the project in the data home and the runtime
        // root: its Agents' workspaces and its executions' scratch files.
        let tenant_home = vak_config::paths::tenant_home_at(
            &shared.clone().into_root(),
            vak_config::paths::LOCAL_TENANT,
        );
        let workspaces: Vec<std::path::PathBuf> = [
            tenant_home.join("workspaces").join(space),
            vak_config::paths::runtime_dir()
                .join("executions")
                .join(space),
        ]
        .into_iter()
        .filter(|dir| dir.is_dir())
        .collect();
        let workspace_files = workspaces.iter().map(|dir| files_under(dir)).sum();
        Ok(AgentReach {
            conversations,
            scopes,
            artifacts,
            documents,
            triggers,
            workspaces,
            workspace_files,
            held,
        })
    }

    /// What erasing everything kept for the project `space` would destroy.
    pub fn project_erasure_preview(&self, space: &str) -> Result<AgentPreview, ErasureError> {
        let _turn = one_at_a_time();
        Ok(Self::agent_preview_of(space, &self.project_reach(space)?))
    }

    /// Erases everything Vakyartha stored for one project: every Agent's
    /// conversations there, what they remember of it, every copy of its
    /// files in the Library, its automations, and its working folders in
    /// the data home. The project's own folder, with its settings, is
    /// never touched, and the project stays known, hidden until it is
    /// opened again. Refused while anything in reach is on hold, and when
    /// a confirmation's digest no longer matches.
    pub fn erase_project(
        &self,
        space: &str,
        digest: Option<&str>,
        cause: Cause,
        actor: Option<vak_session::ids::PrincipalId>,
    ) -> Result<Receipt, ErasureError> {
        let _turn = one_at_a_time();
        let failed = |error: String| ErasureError::Failed(error);
        vak_session::fence::check().map_err(|error| failed(error.to_string()))?;
        let reach = self.project_reach(space)?;
        if reach.held {
            return Err(ErasureError::Held);
        }
        if digest.is_some_and(|digest| digest != Self::agent_preview_of(space, &reach).digest) {
            return Err(ErasureError::StalePreview);
        }
        let receipt =
            self.erase_reach(&reach, "project", space, PROJECT_NOT_REACHED, cause, actor)?;
        let _ = vak_config::spaces::set_forgotten(space, true);
        Ok(receipt)
    }

    /// Puts an artifact on hold or releases it: while held, its drafts do
    /// not go to the trash, none of its versions is erased, and a
    /// conversation whose erasure would take it is refused.
    pub fn hold_artifact(&self, artifact: &str, held: bool) -> Result<(), ErasureError> {
        let failed = |error: String| ErasureError::Failed(error);
        let id = vak_session::ids::ArtifactId::parse(artifact)
            .map_err(|_| ErasureError::NotFound(artifact.to_string()))?;
        if self.artifacts().get(artifact).is_none() {
            return Err(ErasureError::NotFound(artifact.to_string()));
        }
        let tenant = self.tenant_objects()?;
        let scope = crate::artifacts::object_scope(&id);
        if held {
            tenant.hold_scope(&scope)
        } else {
            tenant.release_scope(&scope)
        }
        .map_err(|error| failed(error.to_string()))
    }

    pub fn artifact_held(&self, artifact: &str) -> bool {
        vak_session::ids::ArtifactId::parse(artifact).is_ok_and(|id| {
            self.tenant_objects()
                .is_ok_and(|tenant| tenant.scope_held(&crate::artifacts::object_scope(&id)))
        })
    }

    /// Everything on hold, conversations first.
    pub fn holds(&self) -> Vec<Hold> {
        let Ok(tenant) = self.tenant_objects() else {
            return Vec::new();
        };
        let mut holds: Vec<Hold> = tenant
            .held_scopes()
            .into_iter()
            .map(|scope| {
                if let Some(id) = scope.strip_prefix("conversation:") {
                    Hold {
                        kind: "conversation",
                        id: id.to_string(),
                        name: Some(self.erasure_confirmation(id)),
                    }
                } else if let Some(id) = scope.strip_prefix("artifact:") {
                    Hold {
                        kind: "artifact",
                        id: id.to_string(),
                        name: self.artifacts().get(id).map(|artifact| artifact.name()),
                    }
                } else {
                    Hold {
                        kind: "other",
                        id: scope,
                        name: None,
                    }
                }
            })
            .collect();
        holds.sort_by(|a, b| {
            (a.kind != "conversation", &a.id).cmp(&(b.kind != "conversation", &b.id))
        });
        holds
    }

    /// The people other than the owner whose contributions to
    /// `session_id` are kept under a key of their own.
    pub fn conversation_guests(&self, session_id: &str) -> Vec<String> {
        let Ok(tenant) = self.tenant_objects() else {
            return Vec::new();
        };
        let mut guests: BTreeSet<String> = BTreeSet::new();
        for scope in self.guest_scopes(&tenant, session_id, "") {
            if let Some(principal) = vak_session::objects::contributor_of(&scope) {
                guests.insert(principal.to_string());
            }
        }
        guests.into_iter().collect()
    }

    /// The contributor scopes with a key that `principal` (everyone, when
    /// empty) has in `session_id` and on the artifacts made there.
    fn guest_scopes(
        &self,
        tenant: &TenantObjects,
        session_id: &str,
        principal: &str,
    ) -> Vec<String> {
        let mut owners = vec![session_id.to_string()];
        for artifact in self.artifacts().list() {
            let here = artifact
                .versions
                .iter()
                .any(|version| match &version.source {
                    crate::artifacts::VersionSource::Call { session, .. }
                    | crate::artifacts::VersionSource::Candidate { session, .. } => {
                        session == session_id
                    }
                    crate::artifacts::VersionSource::Person => false,
                });
            if here {
                owners.push(artifact.id.to_string());
            }
        }
        let mut scopes = Vec::new();
        for owner in owners {
            let prefix = contributor_scope(&owner, principal);
            for scope in tenant.scopes_with_prefix(&prefix).unwrap_or_default() {
                // A prefix also matches a longer principal id.
                if principal.is_empty()
                    || vak_session::objects::contributor_of(&scope) == Some(principal)
                {
                    scopes.push(scope);
                }
            }
        }
        scopes
    }

    fn guest_preview_of(
        &self,
        tenant: &TenantObjects,
        session_id: &str,
        principal: &str,
    ) -> Result<(GuestPreview, Vec<String>), ErasureError> {
        let failed = |error: String| ErasureError::Failed(error);
        let state = crate::trash::state(&self.shared_scope(), session_id)
            .map_err(|error| failed(error.to_string()))?;
        if state.erased_at.is_some() {
            return Err(ErasureError::AlreadyErased);
        }
        let scopes = self.guest_scopes(tenant, session_id, principal);
        if principal.is_empty() || scopes.is_empty() {
            return Err(ErasureError::NotFound(format!("{session_id}/{principal}")));
        }
        let own = contributor_scope(session_id, principal);
        let mut digest = Sha256::new();
        for scope in &scopes {
            digest.update(scope.as_bytes());
            digest.update([0]);
        }
        let held = self.conversation_held(session_id)
            || scopes.iter().any(|scope| tenant.scope_held(scope));
        let preview = GuestPreview {
            session_id: session_id.to_string(),
            principal: principal.to_string(),
            digest: digest
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            held,
            messages: scopes.contains(&own),
            artifacts: scopes
                .iter()
                .filter(|scope| **scope != own)
                .filter_map(|scope| scope.split(':').nth(1).map(str::to_string))
                .collect(),
        };
        Ok((preview, scopes))
    }

    /// What erasing `principal`'s contributions to `session_id` would
    /// destroy.
    pub fn guest_erasure_preview(
        &self,
        session_id: &str,
        principal: &str,
    ) -> Result<GuestPreview, ErasureError> {
        let _turn = one_at_a_time();
        let tenant = self.tenant_objects()?;
        Ok(self.guest_preview_of(&tenant, session_id, principal)?.0)
    }

    /// Erases what one person other than the owner wrote in a conversation
    /// and in comments on its artifacts: their keys are destroyed, so each
    /// message reads as removed and each comment is gone, and the
    /// conversation stays whole. Search is rebuilt without their words.
    /// Refused while the conversation or their key is on hold, and when a
    /// person's confirmation carries a digest that no longer matches.
    pub fn erase_guest(
        &self,
        session_id: &str,
        principal: &str,
        digest: Option<&str>,
        cause: Cause,
        actor: Option<vak_session::ids::PrincipalId>,
    ) -> Result<Receipt, ErasureError> {
        let _turn = one_at_a_time();
        let failed = |error: String| ErasureError::Failed(error);
        vak_session::fence::check().map_err(|error| failed(error.to_string()))?;
        let tenant = self.tenant_objects()?;
        let (preview, scopes) = self.guest_preview_of(&tenant, session_id, principal)?;
        if preview.held {
            return Err(ErasureError::Held);
        }
        if digest.is_some_and(|digest| digest != preview.digest) {
            return Err(ErasureError::StalePreview);
        }
        self.destroy_and_record(
            &tenant,
            &scopes,
            Receipt {
                id: format!("ers_{}", uuid::Uuid::now_v7()),
                at: Utc::now(),
                scope: "guest".into(),
                subject: format!("{session_id}/{principal}"),
                cause,
                actor,
                conversations: 0,
                artifacts: 0,
                keys_destroyed: 0,
                keys_digest: String::new(),
                memory_notes_removed: 0,
                search_rows_removed: 0,
                objects_deleted: 0,
                sent_outside: 0,
                not_reached: GUEST_NOT_REACHED.iter().map(ToString::to_string).collect(),
                public_key: String::new(),
                signature: String::new(),
            },
        )
    }

    /// Erases what a connected mail or calendar account returned, in every
    /// conversation of every Agent that read it: the account's key is
    /// destroyed, so each such result reads as a fixed line and the
    /// conversations stay. Search is rebuilt without it. Refused while the
    /// account is still connected to any Agent, and while its key is held.
    pub fn erase_account(
        &self,
        account: &str,
        cause: Cause,
        actor: Option<vak_session::ids::PrincipalId>,
    ) -> Result<Receipt, ErasureError> {
        let _turn = one_at_a_time();
        let failed = |error: String| ErasureError::Failed(error);
        vak_session::fence::check().map_err(|error| failed(error.to_string()))?;
        let tenant = self.tenant_objects()?;
        let scope = vak_session::objects::account_scope(account);
        if tenant.scope_destroyed(&scope) {
            return Err(ErasureError::AlreadyErased);
        }
        if account.is_empty()
            || !tenant
                .scopes_with_prefix(&scope)
                .map_err(|error| failed(error.to_string()))?
                .contains(&scope)
        {
            return Err(ErasureError::NotFound(account.to_string()));
        }
        if tenant.scope_held(&scope) {
            return Err(ErasureError::Held);
        }
        if self.account_connected(account) {
            return Err(ErasureError::StillConnected);
        }
        self.destroy_and_record(
            &tenant,
            &[scope],
            Receipt {
                id: format!("ers_{}", uuid::Uuid::now_v7()),
                at: Utc::now(),
                scope: "account".into(),
                subject: account.to_string(),
                cause,
                actor,
                conversations: 0,
                artifacts: 0,
                keys_destroyed: 0,
                keys_digest: String::new(),
                memory_notes_removed: 0,
                search_rows_removed: 0,
                objects_deleted: 0,
                sent_outside: 0,
                not_reached: ACCOUNT_NOT_REACHED
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                public_key: String::new(),
                signature: String::new(),
            },
        )
    }

    /// Whether any Agent still has `account` connected.
    fn account_connected(&self, account: &str) -> bool {
        let mut agents: Vec<String> = vak_session_agent_homes(&self.shared_scope())
            .into_iter()
            .map(|(agent, _)| agent)
            .collect();
        agents.push("vak".to_string());
        agents.iter().any(|agent| {
            vak_mail_calendar::connection_ledger::ConnectionLedger::for_agent(agent)
                .and_then(|ledger| ledger.read_all())
                .is_ok_and(|accounts| {
                    accounts
                        .iter()
                        .any(|saved| saved.id == account && saved.revoked_at.is_none())
                })
        })
    }

    /// The part every erasure that leaves its conversations shares: the
    /// keys, then the rollups and the search rows that folded what they
    /// protected, the objects nothing holds now, and the signed receipt.
    fn destroy_and_record(
        &self,
        tenant: &TenantObjects,
        scopes: &[String],
        mut receipt: Receipt,
    ) -> Result<Receipt, ErasureError> {
        let failed = |error: String| ErasureError::Failed(error);
        for scope in scopes {
            tenant
                .destroy_scope_key(scope)
                .map_err(|error| failed(error.to_string()))?;
        }
        let shared = self.shared_scope();
        let _ = vak_session::documents::forget(&shared.artifacts_rollup());
        receipt.search_rows_removed = self
            .catalog()
            .map_err(|error| failed(error.to_string()))?
            .rebuild_after_erasure()
            .map_err(|error| failed(error.to_string()))?;
        receipt.objects_deleted = {
            use vak_session::objects::Objects;
            tenant.collect().unwrap_or(0) as u64
        };
        let mut keys = Sha256::new();
        for scope in scopes {
            keys.update(scope.as_bytes());
            keys.update([0]);
        }
        receipt.keys_destroyed = scopes.len() as u64;
        receipt.keys_digest = keys.finalize().iter().map(|b| format!("{b:02x}")).collect();
        let receipt = self.sign_and_record(tenant, receipt)?;
        tracing::info!(
            kind = "erasure",
            outcome = "completed",
            count = receipt.keys_destroyed,
            "contributions were erased"
        );
        Ok(receipt)
    }

    fn install_scopes(tenant: &TenantObjects) -> Result<Vec<String>, ErasureError> {
        tenant
            .scopes_with_prefix("")
            .map_err(|error| ErasureError::Failed(error.to_string()))
    }

    fn install_preview_of(tenant: &TenantObjects, scopes: &[String]) -> InstallPreview {
        let mut digest = Sha256::new();
        for scope in scopes {
            digest.update(scope.as_bytes());
            digest.update([0]);
        }
        let count = |prefix: &str| {
            scopes
                .iter()
                .filter(|scope| scope.starts_with(prefix))
                .count() as u64
        };
        InstallPreview {
            digest: digest
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            held: tenant.held_count() as u64,
            conversations: count("conversation:"),
            artifacts: count("artifact:"),
            keys: scopes.len() as u64,
        }
    }

    /// What erasing the whole install would destroy. Reads only.
    pub fn install_erasure_preview(&self) -> Result<InstallPreview, ErasureError> {
        let tenant = self.tenant_objects()?;
        let scopes = Self::install_scopes(&tenant)?;
        Ok(Self::install_preview_of(&tenant, &scopes))
    }

    /// Erases everything Vakyartha stored: every key is destroyed, every
    /// stored secret is removed, and the data, cache, log and runtime
    /// directories are emptied. All it leaves is a signed receipt, one
    /// file under the data home's `erased/`. Folders a person owns are
    /// not touched. Refused while anything is on hold. The process must
    /// stop afterwards: nothing it holds open exists any more.
    pub fn erase_install(
        &self,
        digest: Option<&str>,
        cause: Cause,
        actor: Option<vak_session::ids::PrincipalId>,
    ) -> Result<Receipt, ErasureError> {
        let _turn = one_at_a_time();
        let failed = |error: String| ErasureError::Failed(error);
        vak_session::fence::check().map_err(|error| failed(error.to_string()))?;
        let tenant = self.tenant_objects()?;
        let scopes = Self::install_scopes(&tenant)?;
        let preview = Self::install_preview_of(&tenant, &scopes);
        if preview.held > 0 {
            return Err(ErasureError::Held);
        }
        if digest.is_some_and(|digest| digest != preview.digest) {
            return Err(ErasureError::StalePreview);
        }
        for scope in &scopes {
            tenant
                .destroy_scope_key(scope)
                .map_err(|error| failed(error.to_string()))?;
        }
        vak_config::credentials::forget_all();
        let keep = vak_config::paths::default_workspace();
        let mut roots = vec![self.shared_root()];
        for root in crate::state::Root::ALL {
            let path = crate::state::root_path(root);
            if root.is_owned() && !roots.contains(&path) {
                roots.push(path);
            }
        }
        let mut not_reached: Vec<String> = INSTALL_NOT_REACHED
            .iter()
            .map(|line| line.to_string())
            .collect();
        let left = roots
            .iter()
            .filter(|root| crate::state::remove_wholesale(root, &keep).is_err())
            .count();
        if left > 0 {
            not_reached.push(format!(
                "Files in {left} of Vakyartha's own directories could not be removed. Their keys are destroyed, so nothing in them can be read."
            ));
        }
        let receipt = Self::sign(
            &tenant,
            Receipt {
                id: format!("ers_{}", uuid::Uuid::now_v7()),
                at: Utc::now(),
                scope: "install".into(),
                subject: "everything".into(),
                cause,
                actor,
                conversations: preview.conversations,
                artifacts: preview.artifacts,
                keys_destroyed: preview.keys,
                keys_digest: preview.digest,
                memory_notes_removed: 0,
                search_rows_removed: 0,
                objects_deleted: 0,
                sent_outside: 0,
                not_reached,
                public_key: String::new(),
                signature: String::new(),
            },
        )?;
        let kept = self.shared_scope().install_receipts();
        std::fs::create_dir_all(&kept).map_err(|error| failed(error.to_string()))?;
        let bytes =
            serde_json::to_vec_pretty(&receipt).map_err(|error| failed(error.to_string()))?;
        std::fs::write(kept.join(format!("{}.json", receipt.id)), bytes)
            .map_err(|error| failed(error.to_string()))?;
        Ok(receipt)
    }

    /// The receipts earlier erasures of the whole install left, oldest
    /// first.
    fn install_receipts(&self) -> Vec<Receipt> {
        let kept = self.shared_scope().install_receipts();
        let mut receipts: Vec<Receipt> = std::fs::read_dir(kept)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| std::fs::read(entry.path()).ok())
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
            .collect();
        receipts.sort_by(|a, b| a.id.cmp(&b.id));
        receipts
    }

    /// Signs a receipt with the tenant's key. The public key is part of
    /// what is signed.
    fn sign(tenant: &TenantObjects, mut receipt: Receipt) -> Result<Receipt, ErasureError> {
        let failed = |error: String| ErasureError::Failed(error);
        let (_, public_key) = tenant
            .sign(b"")
            .map_err(|error| failed(error.to_string()))?;
        receipt.public_key = public_key;
        let (signature, _) = tenant
            .sign(&receipt.signed_bytes())
            .map_err(|error| failed(error.to_string()))?;
        receipt.signature = signature;
        Ok(receipt)
    }

    /// Signs a receipt with the tenant's key and appends it to the
    /// erasures chain. The public key is part of what is signed.
    fn sign_and_record(
        &self,
        tenant: &TenantObjects,
        receipt: Receipt,
    ) -> Result<Receipt, ErasureError> {
        let failed = |error: String| ErasureError::Failed(error);
        let receipt = Self::sign(tenant, receipt)?;
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
        let _turn = one_at_a_time();
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
