//! Projection invariants: nothing the intent kernel derives may widen what a
//! turn is allowed to do (`AGENTS.md` invariant 32), and a grant may only
//! narrow (invariant 3 in docs/design/47).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use vak_core::intent;
use vak_intent::{
    ApprovalCeiling, Authority, Autonomy, DomainSet, Envelope, Escalation, PermissionCeiling,
    Stakes,
};

fn envelope(ceiling: PermissionCeiling) -> Envelope {
    Envelope {
        envelope_id: "env-1".into(),
        granted_by: "nisheeth".into(),
        granted_at: chrono::Utc::now(),
        expires_at: None,
        spend_limit_usd: Some(5.0),
        path_scope: vec!["src/**".into()],
        tool_scope: vec!["edit".into()],
        permission_ceiling: ceiling,
        escalation: Escalation::WaitIndefinitely,
        revoked_at: None,
    }
}

fn resolved(text: &str, authority: &Authority) -> vak_intent::Intent {
    let request = vak_intent::Request {
        text,
        turn_id: "0192f5a0-0000-7000-8000-000000000000",
        surface: vak_intent::Surface::Cli,
        ..vak_intent::Request::default()
    };
    vak_intent::resolve(
        &request,
        &vak_intent::Declared::default(),
        authority,
        &vak_intent::ResolverConfig::default(),
    )
    .intent()
}

/// The single most important property: no delegation, however broad, lets an
/// irreversible action past without a human — not at the turn's ceiling, and
/// not after a grant has narrowed its strands.
#[test]
fn no_grant_lets_irreversible_work_past_a_human() {
    let now = chrono::Utc::now();
    for autonomy in Autonomy::ALL {
        let authority = Authority {
            autonomy,
            attendance: vak_intent::Attendance::Interactive,
        };
        assert_eq!(
            authority.approval_ceiling(Stakes::Irreversible),
            ApprovalCeiling::Ask,
            "{autonomy:?}"
        );
        let intent = resolved("force push to the production branch", &authority);
        for ceiling in PermissionCeiling::ALL {
            let envelopes = intent
                .strands
                .iter()
                .map(|strand| (strand.strand_id.clone(), envelope(ceiling)))
                .collect();
            let narrowed = vak_intent::apply_envelopes(intent.clone(), &envelopes, autonomy, now);
            assert_eq!(
                narrowed.engagement.limits.approval_ceiling,
                ApprovalCeiling::Ask,
                "{autonomy:?}/{ceiling:?}"
            );
            assert_ne!(
                narrowed.engagement.posture.hil,
                vak_intent::HilMode::Envelope,
                "irreversible work never proceeds inside an envelope"
            );
        }
    }
}

/// A grant only ever narrows the turn it applies to.
#[test]
fn a_grant_narrows_and_never_widens_the_turn() {
    let now = chrono::Utc::now();
    let authority = Authority {
        autonomy: Autonomy::Delegated,
        attendance: vak_intent::Attendance::Interactive,
    };
    let intent = resolved("refactor the parser module", &authority);
    for ceiling in PermissionCeiling::ALL {
        let envelopes = intent
            .strands
            .iter()
            .map(|strand| (strand.strand_id.clone(), envelope(ceiling)))
            .collect();
        let narrowed =
            vak_intent::apply_envelopes(intent.clone(), &envelopes, Autonomy::Delegated, now);
        assert!(
            narrowed
                .engagement
                .limits
                .is_at_most(&intent.engagement.limits),
            "{ceiling:?} widened the turn"
        );
        assert_eq!(narrowed.engagement.limits.spend_ceiling_usd, Some(5.0));
    }
}

/// A grant may lower the effective permission mode and may never raise it,
/// composing through the same `capped_by` a gateway channel override uses.
#[test]
fn a_grant_can_only_lower_the_permission_mode() {
    use vak_config::PermissionMode::*;
    for configured in [ReadOnly, WorkspaceWrite, FullAccess] {
        for ceiling in PermissionCeiling::ALL {
            let effective = intent::permission_mode(configured, ceiling);
            assert!(
                effective.rank() <= configured.rank(),
                "{configured:?} + {ceiling:?} widened to {effective:?}"
            );
        }
    }
    // Specifically: a full-access grant does not promote a read-only workspace.
    assert_eq!(
        intent::permission_mode(ReadOnly, PermissionCeiling::FullAccess),
        ReadOnly
    );
}

/// Approval composition takes the stricter of configuration and the
/// engagement's ceiling, in both directions.
#[test]
fn approval_composition_never_loosens_configuration() {
    use vak_config::ApprovalMode::*;
    for configured in [Ask, ApproveSafe, AutoApprove] {
        for ceiling in ApprovalCeiling::ALL {
            let effective = intent::approval_mode(configured, ceiling, true);
            let rank = |mode: vak_config::ApprovalMode| match mode {
                Ask => 0u8,
                ApproveSafe => 1,
                AutoApprove => 2,
            };
            assert!(rank(effective) <= rank(configured));
        }
    }
}

/// A reading decides only what is *loaded*: the surface partitions exactly the
/// admitted tools into loaded and deferred, and can neither add a tool nor
/// lose one.
#[test]
fn a_reading_partitions_the_admitted_tools_and_never_adds_or_drops_one() {
    let admitted = vak_tools::default_tools();
    let names = |defs: &[vak_llm::ToolDefinition]| -> Vec<String> {
        defs.iter().map(|d| d.name.clone()).collect()
    };
    for required in [
        DomainSet::only(["code-exec"]),
        DomainSet::Empty,
        DomainSet::All,
    ] {
        let surface = vak_core::capability::build_tool_surface(
            &admitted,
            &required,
            &Default::default(),
            &Default::default(),
        );
        let mut seen = names(&surface.core);
        seen.extend(names(&surface.deferred));
        seen.sort();
        let mut expected: Vec<String> = admitted.iter().map(|t| t.name().to_string()).collect();
        expected.sort();
        assert_eq!(seen, expected, "{required:?}");
        assert!(
            names(&surface.core).contains(&"read".to_string()),
            "an always-loaded tool is loaded whatever the reading"
        );
    }
}

/// A revoked grant stops narrowing and does not leave a remembered widening
/// behind, and delegation alone buys nothing at the turn level: the gate lets
/// through only what a live grant covers, action by action.
#[test]
fn a_revoked_grant_grants_nothing() {
    let now = chrono::Utc::now();
    let authority = Authority {
        autonomy: Autonomy::Delegated,
        attendance: vak_intent::Attendance::Supervised,
    };
    assert_eq!(
        authority.approval_ceiling(Stakes::Reversible),
        ApprovalCeiling::Ask
    );
    let intent = resolved("refactor the parser module", &authority);
    let mut revoked = envelope(PermissionCeiling::ReadOnly);
    revoked.revoked_at = Some(now);
    let envelopes = intent
        .strands
        .iter()
        .map(|strand| (strand.strand_id.clone(), revoked.clone()))
        .collect();
    assert_eq!(
        vak_intent::apply_envelopes(intent.clone(), &envelopes, Autonomy::Delegated, now),
        intent,
        "a revoked grant must impose no ceiling, and confer nothing either"
    );
}

/// Assuming a default for irreversible work because nobody replied is exactly
/// the autonomy the system exists to prevent.
#[test]
fn a_silent_default_is_refused_for_irreversible_work() {
    let policy = Escalation::AssumeConservative { after_hours: 24 };
    assert!(policy.permitted_for(Stakes::Reversible));
    assert!(policy.permitted_for(Stakes::Costly));
    assert!(!policy.permitted_for(Stakes::Irreversible));
}

/// `vak-config` ranks autonomy names without depending on the intent kernel,
/// so the two rankings must agree. This test sees both and is the only place
/// that can check it.
#[test]
fn config_and_kernel_agree_on_autonomy_ranking() {
    for (lower, higher) in [
        ("manual", "assisted"),
        ("assisted", "delegated"),
        ("delegated", "autonomous"),
    ] {
        let a = Autonomy::parse(lower).unwrap();
        let b = Autonomy::parse(higher).unwrap();
        assert!(a.rank() < b.rank());
        // And the config-side merge must pick the same "less delegated" one.
        let capped = vak_config::ChannelPolicy::cap_autonomy(Some(higher), Some(lower));
        assert_eq!(capped.as_deref(), Some(lower));
    }
}

/// A channel ceiling composes downward with the workspace grant.
#[test]
fn a_channel_ceiling_caps_the_workspace_grant() {
    for granted in Autonomy::ALL {
        for ceiling in Autonomy::ALL {
            let effective = granted.capped_by(ceiling);
            assert!(effective.rank() <= granted.rank());
            assert!(effective.rank() <= ceiling.rank());
        }
    }
}
