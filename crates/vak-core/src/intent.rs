//! Wiring the intent kernel into the runtime.
//!
//! `vak-intent` is a pure decision layer that knows nothing about sessions,
//! providers, or the permission engine. This module is the seam: it gathers
//! the facts a reading needs, runs the cascade, and projects the resulting
//! engagement onto the runtime knobs it governs.
//!
//! # The projection contract
//!
//! Every function here takes a baseline and returns something no wider.
//! Nothing in this module grants: an approval ceiling takes the stricter of
//! itself and the configured mode, a permission ceiling caps the mode, and a
//! spend ceiling takes the smaller. What a reading predicts decides only
//! which admitted tools are loaded, never what is possible: the route
//! ladder, the turn budget and delegation are the operator's, whatever the
//! request looked like. `debug_assert`s state the property at each site and
//! `tests/intent_projection.rs` proves it.
//!
//! Getting a reading wrong must therefore be able to make vak *more*
//! cautious, and never less.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use vak_intent::{
    ApprovalCeiling, Authority, Autonomy, Declared, Engagement, HistoryFacts, Intent, Limits,
    PermissionCeiling, Request, Resolution, ResolverConfig, Surface as IntentSurface,
    WorkspaceFacts,
};

use crate::Surface;

/// Map the runtime's surface onto the kernel's.
///
/// `Unknown` reads as `Server` rather than `Cli`: a caller that did not say
/// where it was is not evidence that a human is sitting there, and assuming
/// one would let an unattended embedder raise gates nobody answers.
pub fn intent_surface(surface: &Surface) -> IntentSurface {
    match surface {
        Surface::Cli | Surface::Terminal => IntentSurface::Cli,
        // The web client IS the desktop client, in a tab: the same panes,
        // the same approval cards, the same person watching. Reading it as
        // `Server` would treat an attended session as an unattended
        // embedder and stop raising gates that someone is right there to
        // answer (docs/design/48-web-client.md).
        Surface::Desktop | Surface::Web => IntentSurface::Desktop,
        Surface::Server | Surface::Unknown => IntentSurface::Server,
        Surface::Chat { .. } => IntentSurface::Chat,
        Surface::Background => IntentSurface::Cron,
        Surface::Worker => IntentSurface::Worker,
    }
}

/// Configured autonomy, parsed once with a safe fallback.
pub fn configured_autonomy(config: &vak_config::Config) -> Autonomy {
    Autonomy::parse(&config.intent.autonomy).unwrap_or(Autonomy::Assisted)
}

/// Build the resolver configuration from the workspace's settings.
pub fn resolver_config(config: &vak_config::Config) -> ResolverConfig {
    ResolverConfig {
        enabled: config.intent.enabled,
        accept_confidence: config.intent.accept_confidence,
        provisional_confidence: config.intent.provisional_confidence,
        slice_capabilities: config.intent.slice_capabilities,
        allow_escalation: config.intent.escalate != "none",
    }
}

/// Facts about the workspace, gathered cheaply.
///
/// Deliberately shallow: this runs before every turn, so it may not walk the
/// tree or shell out. `.git` presence and index mtime are enough to separate
/// "a repository with work in progress" from "an empty directory", which is
/// all the reading needs.
pub fn workspace_facts(cwd: &std::path::Path) -> WorkspaceFacts {
    let git_dir = cwd.join(".git");
    let is_repo = git_dir.exists();
    // A modified index is a cheap, dependency-free proxy for "there is
    // uncommitted work here": it is touched by `git add`/`git rm` and by most
    // porcelain that changes the tree. It can miss purely-unstaged edits, so
    // it is used only to *raise* caution, never to lower it.
    let has_uncommitted_changes = is_repo
        && std::fs::metadata(git_dir.join("index"))
            .and_then(|meta| meta.modified())
            .ok()
            .zip(
                std::fs::metadata(git_dir.join("HEAD"))
                    .and_then(|meta| meta.modified())
                    .ok(),
            )
            .is_some_and(|(index, head)| index > head);
    WorkspaceFacts {
        is_repo,
        has_uncommitted_changes,
    }
}

/// What the session so far says about the next request: how many messages
/// it holds, what the last turn was read as, and which threads are open.
///
/// One function for the turn and for every preview of it (`vak intent
/// explain`, `GET /intent/explain`), so a preview reads a request exactly as
/// the turn would.
pub fn history_facts(session: &vak_session::SessionLog) -> HistoryFacts {
    let chain = session.chain_to_root();
    let previous_act = chain.iter().rev().find_map(|entry| match &entry.payload {
        vak_session::EntryPayload::Intent(record) => Some(record.reading.act),
        _ => None,
    });
    let turn_index = chain
        .iter()
        .filter(|entry| matches!(entry.payload, vak_session::EntryPayload::Message(_)))
        .count();
    HistoryFacts {
        previous_act,
        turn_index,
        open_threads: open_threads(session),
    }
}

/// Resolve one turn's intent.
///
/// `turn_id` is the id the host minted for this turn (a UUIDv7); strand and
/// thread ids derive from it, so a thread — and the commitment keyed by it —
/// is unique across turns and sessions. A preview passes `""` and gets
/// positional ids, which it never persists.
///
/// Returns the resolution rather than an `Intent` so the caller can decide
/// whether to spend a classification dispatch on an
/// [`Resolution::Escalate`]. The partial inside it is always safe to use.
#[allow(clippy::too_many_arguments)]
pub fn resolve_turn(
    text: &str,
    turn_id: &str,
    surface: &Surface,
    attachments: &[vak_intent::Attachment],
    workspace: WorkspaceFacts,
    history: HistoryFacts,
    declared: &Declared,
    authority: &Authority,
    config: &ResolverConfig,
) -> Resolution {
    // An explicit `/goal fix` or `/goal replace` is the only way a strand
    // becomes a correction or a replacement of earlier work.
    let (text, lineage_hint) = match vak_intent::parse_command(text) {
        Some(vak_intent::Command::GoalFix { text }) => {
            (text, Some(vak_intent::LineageHint::Corrects))
        }
        Some(vak_intent::Command::GoalReplace { text }) => {
            (text, Some(vak_intent::LineageHint::Replaces))
        }
        _ => (text.to_string(), None),
    };
    let request = Request {
        text: &text,
        turn_id,
        surface: intent_surface(surface),
        attachments,
        workspace,
        history,
        attendance_override: Some(authority.attendance),
        lineage_hint,
    };
    vak_intent::resolve(&request, declared, authority, config)
}

/// The threads still open in a session, for strand lineage.
///
/// A thread is open while its most recent strand is not `Replaces`d and no
/// later turn closed it; the last `MAX_OPEN_THREADS` distinct threads are
/// kept, newest first, so a long session does not link every request to
/// something said an hour ago.
pub fn open_threads(session: &vak_session::SessionLog) -> Vec<vak_intent::ThreadFact> {
    const MAX_OPEN_THREADS: usize = 12;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut replaced: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    for entry in session.chain_to_root().into_iter().rev() {
        let vak_session::EntryPayload::Intent(record) = &entry.payload else {
            continue;
        };
        for strand in record.strands.iter().rev() {
            if let vak_intent::Lineage::Replaces { thread_id } = &strand.lineage {
                replaced.insert(thread_id.clone());
            }
            if replaced.contains(&strand.thread_id) || !seen.insert(strand.thread_id.clone()) {
                continue;
            }
            out.push(vak_intent::ThreadFact {
                thread_id: strand.thread_id.clone(),
                act: strand.reading.act,
                domains: strand.reading.domains.clone(),
                keywords: vak_intent::strand::keywords(&strand.text),
            });
            if out.len() >= MAX_OPEN_THREADS {
                break;
            }
        }
        if out.len() >= MAX_OPEN_THREADS {
            break;
        }
    }
    // Oldest first, so "the most recent open thread" is `last()`.
    out.reverse();
    out
}

// --------------------------------------------------------- projections ---

/// The approval mode this turn should run under.
///
/// Takes the **stricter** of the configured mode and the engagement's ceiling.
/// Intent can therefore force a gate the configuration would have skipped, and
/// can never skip one the configuration wanted.
pub fn approval_mode(
    configured: vak_config::ApprovalMode,
    ceiling: ApprovalCeiling,
    posture_enabled: bool,
) -> vak_config::ApprovalMode {
    if !posture_enabled {
        return configured;
    }
    let configured_rank = match configured {
        vak_config::ApprovalMode::Ask => 0,
        vak_config::ApprovalMode::ApproveSafe => 1,
        vak_config::ApprovalMode::AutoApprove => 2,
    };
    let effective = configured_rank.min(ceiling.rank());
    let result = match effective {
        0 => vak_config::ApprovalMode::Ask,
        1 => vak_config::ApprovalMode::ApproveSafe,
        _ => vak_config::ApprovalMode::AutoApprove,
    };
    debug_assert!(
        approval_rank(result) <= configured_rank,
        "intent loosened the configured approval mode"
    );
    result
}

fn approval_rank(mode: vak_config::ApprovalMode) -> u8 {
    match mode {
        vak_config::ApprovalMode::Ask => 0,
        vak_config::ApprovalMode::ApproveSafe => 1,
        vak_config::ApprovalMode::AutoApprove => 2,
    }
}

/// The permission mode this turn should run under.
///
/// Uses the existing `PermissionMode::capped_by`, which is the one place in
/// the codebase that reconciles a requested grant against a ceiling. An
/// envelope narrows through the same door as a gateway channel override.
pub fn permission_mode(
    configured: vak_config::PermissionMode,
    ceiling: PermissionCeiling,
) -> vak_config::PermissionMode {
    let ceiling = match ceiling {
        PermissionCeiling::ReadOnly => vak_config::PermissionMode::ReadOnly,
        PermissionCeiling::WorkspaceWrite => vak_config::PermissionMode::WorkspaceWrite,
        PermissionCeiling::FullAccess => vak_config::PermissionMode::FullAccess,
    };
    let result = configured.capped_by(ceiling);
    debug_assert!(
        result.rank() <= configured.rank(),
        "intent widened the permission mode"
    );
    result
}

/// The spend ceiling for this run: the smaller of the configured cap and the
/// engagement's.
pub fn spend_ceiling(configured: Option<f64>, engagement: Option<f64>) -> Option<f64> {
    match (configured, engagement) {
        (None, other) => other,
        (this, None) => this,
        (Some(a), Some(b)) => Some(a.min(b)),
    }
}

/// Demand facts for the route ladder's objective selection.
///
/// This is the call that turns the existing router on. `plan_route_ladder` has
/// always passed zeros here, so every session scored identical demand and the
/// objective was effectively constant.
pub fn demand_input(
    engagement: &Engagement,
    estimated_input_tokens: u64,
    output_budget_tokens: u64,
    tool_count: usize,
) -> vak_llm::DemandInput {
    vak_llm::DemandInput {
        estimated_input_tokens,
        output_budget_tokens,
        tool_count,
        structured_output: engagement.posture.demand.structured_output,
        reasoning_required: engagement.posture.demand.reasoning_required,
        evidence_required: engagement.posture.demand.evidence_required,
    }
}

/// Whether a route leg can serve this turn's modalities.
///
/// Model catalogues are discovered, never hardcoded (`AGENTS.md` invariant 9),
/// and multimodal support is a property of the model rather than of our source
/// tree. So capability comes from operator-declared hints — the same mechanism
/// `route.quality_hints` already uses — and a turn with no declared hints
/// treats every leg as capable rather than inventing a restriction.
pub fn leg_supports_modalities(
    model: &str,
    required: &BTreeSet<vak_intent::Modality>,
    hints: &[String],
) -> bool {
    if required.is_empty() || hints.is_empty() {
        return true;
    }
    let model = model.to_ascii_lowercase();
    hints
        .iter()
        .any(|hint| model.contains(&hint.to_ascii_lowercase()))
}

/// An approver that parks a gate nobody here can answer.
///
/// The `Defer` human-in-the-loop mode (docs/design/47-commitment-kernel.md):
/// when the surface cannot answer and the work is durable enough to own a
/// commitment, an `Ask` becomes an inbox entry and a `Suspended { Human }`
/// event on the commitment, and the turn still fails closed — nothing
/// happens without the answer, but the work survives to be resumed. On a
/// surface that *can* answer, this is transparent.
pub struct DeferringApprover {
    inner: Option<std::sync::Arc<dyn vak_agent::Approver>>,
    shared_home: std::path::PathBuf,
    sessions_home: std::path::PathBuf,
    session_id: String,
    commitment_id: String,
    escalation: vak_intent::Escalation,
}

impl DeferringApprover {
    pub fn new(
        inner: Option<std::sync::Arc<dyn vak_agent::Approver>>,
        shared_home: std::path::PathBuf,
        sessions_home: std::path::PathBuf,
        session_id: String,
        commitment_id: String,
        escalation: vak_intent::Escalation,
    ) -> Self {
        DeferringApprover {
            inner,
            shared_home,
            sessions_home,
            session_id,
            commitment_id,
            escalation,
        }
    }
}

#[async_trait::async_trait]
impl vak_agent::Approver for DeferringApprover {
    async fn approve(&self, tool: &str, args_json: &str, reason: &str) -> bool {
        if let Some(inner) = &self.inner
            && inner.answerable()
        {
            return inner.approve(tool, args_json, reason).await;
        }
        let question = format!("`{tool}` needs approval: {reason}");
        let body = format!("{question}\n\nArguments:\n{args_json}");
        match crate::commitments::defer_for_human(
            &self.sessions_home,
            &self.commitment_id,
            &question,
            None,
            self.escalation.clone(),
        ) {
            Ok(question_id) => {
                let _ = crate::inbox::record(
                    &self.shared_home,
                    crate::inbox::Kind::ApprovalPending,
                    &format!("Decision needed for {}", self.commitment_id),
                    &format!(
                        "{body}\n\nquestion: {question_id}\ncommitment: {}",
                        self.commitment_id
                    ),
                    Some(&self.session_id),
                    None,
                );
            }
            Err(error) => {
                let _ = crate::inbox::record(
                    &self.shared_home,
                    crate::inbox::Kind::ApprovalDenied,
                    &format!("Gate denied for {}", self.commitment_id),
                    &format!("{body}\n\ncould not suspend the commitment: {error}"),
                    Some(&self.session_id),
                    None,
                );
            }
        }
        // Fail closed, exactly as before: the answer arrives through the
        // inbox and the commitment resumes from there.
        false
    }

    fn answerable(&self) -> bool {
        self.inner.as_ref().is_some_and(|inner| inner.answerable())
    }
}

/// Pre-authorization from the envelopes on the commitments this turn works
/// on, consulted at an `Ask` gate (`vak_agent::EnvelopeCheck`).
///
/// The grant is read from the ledger on every call, never captured, so a
/// revocation, an expiry, a closure or an exhausted spend limit takes effect
/// at the very next gate (invariant 11 applied to delegation). A call is
/// covered only when every path it names is inside the envelope's scope and
/// its tool is in the envelope's tool list, if it has one
/// (`Envelope::covers`). The caller installs this only for a delegated turn
/// with nothing irreversible in it: irreversible work reaches a human
/// whatever was delegated (invariant 32).
pub fn envelope_check(
    sessions_home: PathBuf,
    commitment_ids: Vec<String>,
    workspace: PathBuf,
) -> vak_agent::EnvelopeCheck {
    std::sync::Arc::new(move |tool, input| {
        let ledger = vak_commit::CommitmentLedger::new(&sessions_home);
        let paths = action_paths(input, &workspace);
        let now = chrono::Utc::now();
        commitment_ids.iter().find_map(|id| {
            let commitment = ledger.get(id).ok().flatten()?;
            if commitment.phase.is_terminal() {
                return None;
            }
            let envelope = commitment.envelope?;
            let within_budget = envelope
                .spend_limit_usd
                .is_none_or(|limit| limit.is_finite() && commitment.spend_usd < limit);
            (within_budget && envelope.is_live(now) && envelope.covers(tool, &paths))
                .then_some(envelope.envelope_id)
        })
    })
}

/// The paths a call names, made workspace-relative where they are inside the
/// workspace. Anything else is passed through as written, where
/// `Envelope::covers` treats it as uncoverable.
fn action_paths(input: &serde_json::Value, workspace: &Path) -> Vec<String> {
    let mut raw: Vec<&str> = ["path", "file_path"]
        .iter()
        .filter_map(|key| input.get(*key).and_then(serde_json::Value::as_str))
        .collect();
    if let Some(paths) = input.get("paths").and_then(serde_json::Value::as_array) {
        raw.extend(paths.iter().filter_map(serde_json::Value::as_str));
    }
    let roots: Vec<PathBuf> = [Some(workspace.to_path_buf()), workspace.canonicalize().ok()]
        .into_iter()
        .flatten()
        .collect();
    raw.into_iter()
        .map(|path| {
            let candidate = Path::new(path);
            if candidate.is_absolute()
                && let Some(relative) = roots
                    .iter()
                    .find_map(|root| candidate.strip_prefix(root).ok())
            {
                return relative.to_string_lossy().into_owned();
            }
            path.to_string()
        })
        .collect()
}

/// The engagement's contribution to the prompt, as a runtime section.
///
/// Code-owned, like the `Surface:` line: it sits beside the other generated
/// sections so no editable layer can name it, rewrite it, or delete it.
pub fn intent_section(intent: &Intent) -> String {
    match intent.engagement.posture.note.as_deref() {
        Some(note) if !note.trim().is_empty() => note.to_string(),
        _ => String::new(),
    }
}

/// Assert the whole projection narrows. Used by tests and debug builds.
pub fn projection_is_narrowing(limits: &Limits) -> bool {
    limits.is_at_most(&Limits::unrestricted())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Intent may force a gate the configuration skipped; it may never skip
    /// one the configuration wanted.
    #[test]
    fn approval_composition_only_tightens() {
        use vak_config::ApprovalMode::*;
        for configured in [Ask, ApproveSafe, AutoApprove] {
            for ceiling in ApprovalCeiling::ALL {
                let result = approval_mode(configured, ceiling, true);
                assert!(
                    approval_rank(result) <= approval_rank(configured),
                    "{configured:?} + {ceiling:?} loosened to {result:?}"
                );
            }
        }
        assert_eq!(
            approval_mode(AutoApprove, ApprovalCeiling::Ask, true),
            Ask,
            "an irreversible turn must reach a human even under auto-approve"
        );
    }

    #[test]
    fn disabling_posture_leaves_the_configured_approval_mode_untouched() {
        use vak_config::ApprovalMode::*;
        assert_eq!(
            approval_mode(AutoApprove, ApprovalCeiling::Ask, false),
            AutoApprove
        );
    }

    #[test]
    fn permission_composition_only_tightens() {
        use vak_config::PermissionMode::*;
        for configured in [ReadOnly, WorkspaceWrite, FullAccess] {
            for ceiling in PermissionCeiling::ALL {
                let result = permission_mode(configured, ceiling);
                assert!(result.rank() <= configured.rank());
            }
        }
        assert_eq!(
            permission_mode(FullAccess, PermissionCeiling::ReadOnly),
            ReadOnly
        );
        // And an envelope cannot promote a read-only workspace.
        assert_eq!(
            permission_mode(ReadOnly, PermissionCeiling::FullAccess),
            ReadOnly
        );
    }

    fn grant(
        home: &Path,
        workspace: &Path,
        path_scope: &[&str],
        tool_scope: &[&str],
    ) -> (String, vak_intent::Envelope) {
        let ledger = vak_commit::CommitmentLedger::new(home);
        let id = ledger
            .open_commitment(vak_commit::spec_from_reading(
                "keep the docs current",
                vak_intent::Reading::general(),
                Vec::new(),
                workspace.to_path_buf(),
                vak_commit::Economics::default(),
            ))
            .unwrap();
        let envelope = vak_intent::Envelope {
            envelope_id: "env-1".into(),
            granted_by: "owner".into(),
            granted_at: chrono::Utc::now(),
            expires_at: None,
            spend_limit_usd: None,
            path_scope: path_scope.iter().map(|s| s.to_string()).collect(),
            tool_scope: tool_scope.iter().map(|s| s.to_string()).collect(),
            permission_ceiling: PermissionCeiling::WorkspaceWrite,
            escalation: vak_intent::Escalation::WaitIndefinitely,
            revoked_at: None,
        };
        ledger
            .append(&vak_commit::Event::new(
                &id,
                vak_commit::EventKind::EnvelopeGranted {
                    envelope: Box::new(envelope.clone()),
                },
            ))
            .unwrap();
        (id, envelope)
    }

    #[test]
    fn an_envelope_covers_only_what_its_scope_names() {
        let home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let (id, _) = grant(
            home.path(),
            workspace.path(),
            &["docs/**"],
            &["write", "edit"],
        );
        let check = envelope_check(
            home.path().to_path_buf(),
            vec![id],
            workspace.path().to_path_buf(),
        );
        let covered = |tool: &str, input: serde_json::Value| check(tool, &input).is_some();
        assert!(covered(
            "edit",
            serde_json::json!({"path": "docs/guide.md"})
        ));
        // An absolute path inside the workspace is the same file.
        let absolute = workspace.path().join("docs/guide.md");
        assert!(covered("write", serde_json::json!({"path": absolute})));
        // Outside the path scope, outside the tool scope, climbing out, or
        // naming no path at all: not covered, so the gate asks as before.
        assert!(!covered("edit", serde_json::json!({"path": "src/main.rs"})));
        assert!(!covered(
            "bash",
            serde_json::json!({"command": "rm -rf docs"})
        ));
        assert!(!covered(
            "edit",
            serde_json::json!({"path": "docs/../.env"})
        ));
        assert!(!covered("edit", serde_json::json!({"path": "/etc/hosts"})));
        assert!(!covered("edit", serde_json::json!({})));
    }

    /// The grant is read at every gate: a revocation applies to the very next
    /// call, not to the next session.
    #[test]
    fn a_revoked_envelope_stops_covering_at_the_next_gate() {
        let home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let (id, envelope) = grant(home.path(), workspace.path(), &[], &[]);
        let check = envelope_check(
            home.path().to_path_buf(),
            vec![id.clone()],
            workspace.path().to_path_buf(),
        );
        let input = serde_json::json!({"path": "notes.md"});
        assert_eq!(check("write", &input), Some(envelope.envelope_id.clone()));
        vak_commit::CommitmentLedger::new(home.path())
            .append(&vak_commit::Event::new(
                &id,
                vak_commit::EventKind::EnvelopeRevoked {
                    envelope_id: envelope.envelope_id,
                    by: "owner".into(),
                },
            ))
            .unwrap();
        assert_eq!(check("write", &input), None);
    }

    #[test]
    fn spend_ceilings_take_the_smaller() {
        assert_eq!(spend_ceiling(Some(5.0), Some(1.0)), Some(1.0));
        assert_eq!(spend_ceiling(Some(1.0), Some(5.0)), Some(1.0));
        assert_eq!(spend_ceiling(None, Some(2.0)), Some(2.0));
        assert_eq!(spend_ceiling(Some(2.0), None), Some(2.0));
        assert_eq!(spend_ceiling(None, None), None);
    }

    /// An unnamed surface must not be mistaken for a human at a terminal.
    #[test]
    fn an_unknown_surface_is_not_assumed_interactive() {
        assert_eq!(intent_surface(&Surface::Unknown), IntentSurface::Server);
        assert_eq!(intent_surface(&Surface::Background), IntentSurface::Cron);
        assert_eq!(
            intent_surface(&Surface::Chat {
                channel: "telegram".into()
            }),
            IntentSurface::Chat
        );
    }

    #[test]
    fn modality_filtering_abstains_without_declared_hints() {
        let required = BTreeSet::from([vak_intent::Modality::Image]);
        // No hints configured: we do not know, so we do not restrict.
        assert!(leg_supports_modalities("some-model", &required, &[]));
        // With hints, only matching legs qualify.
        let hints = vec!["vision".to_string()];
        assert!(leg_supports_modalities("big-vision-1", &required, &hints));
        assert!(!leg_supports_modalities("text-only-2", &required, &hints));
        // A text-only turn is unconstrained either way.
        assert!(leg_supports_modalities(
            "text-only-2",
            &BTreeSet::new(),
            &hints
        ));
    }
}
