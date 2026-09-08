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
//! Nothing in this module grants: a capability slice intersects the admitted
//! packet, an approval ceiling takes the stricter of itself and the configured
//! mode, a ladder limit truncates a prefix, and a spend ceiling takes the
//! smaller. `debug_assert`s state the property at each site and
//! `tests/intent_projection.rs` proves it.
//!
//! Getting a reading wrong must therefore be able to make vak *less* capable
//! or *more* cautious, and never the reverse.

use std::collections::BTreeSet;

use vak_intent::{
    ApprovalCeiling, Authority, Autonomy, Declared, Engagement, Intent, Limits, PermissionCeiling,
    Request, Resolution, ResolverConfig, Surface as IntentSurface, WorkspaceFacts,
};
use vak_session::types::CapabilityDescriptor;

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
        Surface::Subagent => IntentSurface::Subagent,
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
        recent_paths: Vec::new(),
    }
}

/// Resolve one turn's intent.
///
/// Returns the resolution rather than an `Intent` so the caller can decide
/// whether to spend a classification dispatch on an
/// [`Resolution::Escalate`]. The partial inside it is always safe to use.
#[allow(clippy::too_many_arguments)]
pub fn resolve_turn(
    text: &str,
    surface: &Surface,
    attachments: &[vak_intent::Attachment],
    workspace: WorkspaceFacts,
    history: vak_intent::HistoryFacts,
    declared: &Declared,
    authority: &Authority,
    config: &ResolverConfig,
) -> Resolution {
    let request = Request {
        text,
        surface: intent_surface(surface),
        attachments,
        workspace,
        history,
        attendance_override: Some(authority.attendance),
    };
    vak_intent::resolve(&request, declared, authority, config)
}

// --------------------------------------------------------- projections ---

/// Narrow an admitted capability packet to what this turn plausibly needs.
///
/// Subtractive only: a slice can never introduce a capability, and the frozen
/// packet is untouched — this narrows *advertisement and dispatch* for one
/// turn, which is why a wider reading on the next turn restores the full set
/// without needing a new session.
///
/// Matching is by declared domain, not by tool name. The old shape compared
/// against a static list of built-in names, so a capability the user had
/// installed could never be matched — every integration needed a harness
/// edit, and only got one after somebody reported a confidently wrong answer.
/// Here a capability that declares `serves` is kept when it serves something
/// the turn needs, and one that declares nothing is always kept: slicing
/// saves context, it does not enforce policy, so failing open is correct.
pub fn slice_capabilities(
    admitted: &[CapabilityDescriptor],
    required_domains: &std::collections::BTreeSet<String>,
    declared_serves: &std::collections::BTreeMap<String, Vec<String>>,
) -> Vec<CapabilityDescriptor> {
    if required_domains.is_empty() {
        return admitted.to_vec();
    }
    let required: std::collections::BTreeSet<crate::capability::Domain> = required_domains
        .iter()
        .map(|name| crate::capability::Domain::parse(name))
        .collect();
    let narrowed: Vec<CapabilityDescriptor> = admitted
        .iter()
        .filter(|capability| keep_capability(capability, &required, declared_serves))
        .cloned()
        .collect();
    debug_assert!(
        narrowed.len() <= admitted.len(),
        "capability slice grew the admitted packet"
    );
    narrowed
}

/// Whether one admitted capability survives the slice.
///
/// Only model-callable tools are sliced. Skills, MCP servers, hooks and
/// commands stay: a skill is already progressively disclosed by its loader, an
/// MCP server is already lazy, and hooks fire on lifecycle events that have
/// nothing to do with what the user asked for. Slicing those would spend risk
/// for no context saving.
fn keep_capability(
    capability: &CapabilityDescriptor,
    required: &std::collections::BTreeSet<crate::capability::Domain>,
    declared_serves: &std::collections::BTreeMap<String, Vec<String>>,
) -> bool {
    use vak_session::types::CapabilityKind;
    if capability.kind != CapabilityKind::Tool {
        return true;
    }
    // The orientation floor: vak's own tools for looking at what is in front
    // of it. Kept by name because it must hold even for acts whose domains
    // would exclude them, and it never grows when a user installs something.
    if vak_intent::ORIENTATION_FLOOR.contains(&capability.name.as_str()) {
        return true;
    }
    match declared_serves.get(&capability.name) {
        // Undeclared fails open.
        None => true,
        Some(serves) => {
            let mine = crate::capability::Domain::parse_list(serves);
            mine.intersection(required).next().is_some()
        }
    }
}

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

/// Truncate the frozen route ladder to the engagement's prefix.
///
/// A prefix of a frozen ladder is still the frozen ladder: dispatch stays
/// inside the committed contract, ordering is untouched, and replay reproduces
/// the same legs in the same order. Extending or reordering would break the
/// frozen-contract invariant, so neither is offered.
pub fn limit_ladder<T: Clone>(ladder: &[T], limit: Option<usize>) -> Vec<T> {
    match limit {
        // Never truncate to nothing: a ladder with no legs cannot dispatch at
        // all, which would turn a narrowing into an outage.
        Some(limit) if limit > 0 => ladder.iter().take(limit).cloned().collect(),
        _ => ladder.to_vec(),
    }
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
    use vak_session::types::{CapabilityInvocation, CapabilityKind};

    fn tool(name: &str) -> CapabilityDescriptor {
        CapabilityDescriptor {
            name: name.into(),
            kind: CapabilityKind::Tool,
            invocation: CapabilityInvocation::ModelTool,
            description: String::new(),
            source: None,
            digest: None,
            provenance: None,
            configuration: serde_json::Value::Null,
        }
    }

    fn skill(name: &str) -> CapabilityDescriptor {
        CapabilityDescriptor {
            kind: CapabilityKind::Skill,
            invocation: CapabilityInvocation::SkillLoader,
            ..tool(name)
        }
    }

    fn domains(values: &[&str]) -> std::collections::BTreeSet<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    fn serves(pairs: &[(&str, &[&str])]) -> std::collections::BTreeMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(name, ds)| {
                (
                    name.to_string(),
                    ds.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
                )
            })
            .collect()
    }

    /// A slice can only ever remove.
    #[test]
    fn a_slice_never_introduces_a_capability() {
        let admitted = vec![tool("read"), tool("bash")];
        let declared = serves(&[("read", &["filesystem"]), ("bash", &["code-exec"])]);
        let narrowed = slice_capabilities(&admitted, &domains(&["filesystem"]), &declared);
        let names: Vec<&str> = narrowed.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["read"]);
    }

    #[test]
    fn slicing_leaves_skills_and_servers_alone() {
        let admitted = vec![tool("read"), tool("bash"), skill("review")];
        let declared = serves(&[("read", &["filesystem"]), ("bash", &["code-exec"])]);
        let narrowed = slice_capabilities(&admitted, &domains(&["filesystem"]), &declared);
        assert!(narrowed.iter().any(|c| c.name == "review"));
        assert!(!narrowed.iter().any(|c| c.name == "bash"));
    }

    #[test]
    fn no_required_domains_is_the_identity() {
        let admitted = vec![tool("read"), skill("review")];
        assert_eq!(
            slice_capabilities(
                &admitted,
                &std::collections::BTreeSet::new(),
                &std::collections::BTreeMap::new()
            )
            .len(),
            admitted.len()
        );
    }

    /// The defect this whole mechanism was rebuilt around: a capability the
    /// operator installed must be reachable without a harness edit.
    #[test]
    fn an_undeclared_tool_is_never_sliced_away() {
        // `bash` declares code-exec and is out; the unclassified tool has no
        // declaration and must survive, because a capability the operator
        // installed can never appear in a harness-side list.
        let admitted = vec![tool("bash"), tool("some_installed_thing")];
        let declared = serves(&[("bash", &["code-exec"])]);
        let narrowed = slice_capabilities(&admitted, &domains(&["live-data"]), &declared);
        let names: Vec<&str> = narrowed.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["some_installed_thing"],
            "undeclared must fail open"
        );
    }

    #[test]
    fn the_orientation_floor_survives_any_slice() {
        let admitted = vec![tool("read"), tool("glob"), tool("bash")];
        let declared = serves(&[
            ("read", &["filesystem"]),
            ("glob", &["filesystem"]),
            ("bash", &["code-exec"]),
        ]);
        let narrowed = slice_capabilities(&admitted, &domains(&["live-data"]), &declared);
        let names: Vec<&str> = narrowed.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"read"), "orientation floor kept by name");
        assert!(names.contains(&"glob"));
        assert!(!names.contains(&"bash"));
    }

    /// The weather case, end to end at this layer.
    #[test]
    fn a_live_data_turn_keeps_the_mcp_broker() {
        let admitted = vec![tool("mcp"), tool("bash")];
        let declared = serves(&[("mcp", &["live-data", "web"]), ("bash", &["code-exec"])]);
        let narrowed = slice_capabilities(&admitted, &domains(&["live-data"]), &declared);
        assert!(
            narrowed.iter().any(|c| c.name == "mcp"),
            "a configured search server is unreachable without this"
        );
    }

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

    #[test]
    fn a_ladder_limit_takes_a_prefix_and_never_empties_it() {
        let ladder = vec!["a", "b", "c"];
        assert_eq!(limit_ladder(&ladder, Some(2)), vec!["a", "b"]);
        assert_eq!(limit_ladder(&ladder, None), ladder);
        // A zero limit would make dispatch impossible; a narrowing must not
        // become an outage.
        assert_eq!(limit_ladder(&ladder, Some(0)), ladder);
        // Over-long limits are harmless.
        assert_eq!(limit_ladder(&ladder, Some(99)), ladder);
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
