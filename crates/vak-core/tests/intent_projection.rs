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

/// The single most important property: no delegation, however broad, lets an
/// irreversible action past without a human.
#[test]
fn no_grant_lets_irreversible_work_past_a_human() {
    let now = chrono::Utc::now();
    for autonomy in Autonomy::ALL {
        for ceiling in PermissionCeiling::ALL {
            let authority = Authority {
                autonomy,
                attendance: vak_intent::Attendance::Interactive,
                envelope: Some(envelope(ceiling)),
            };
            for in_envelope in [true, false] {
                assert_eq!(
                    authority.approval_ceiling(Stakes::Irreversible, now, in_envelope),
                    ApprovalCeiling::Ask,
                    "{autonomy:?}/{ceiling:?} in_envelope={in_envelope}"
                );
            }
        }
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

/// A slice intersects the admitted packet and can only ever subtract.
#[test]
fn a_capability_slice_only_ever_subtracts() {
    use vak_session::types::{CapabilityDescriptor, CapabilityInvocation, CapabilityKind};
    let admitted: Vec<CapabilityDescriptor> = ["bash", "grep"]
        .into_iter()
        .map(|name| CapabilityDescriptor {
            name: name.into(),
            kind: CapabilityKind::Tool,
            invocation: CapabilityInvocation::ModelTool,
            description: String::new(),
            source: None,
            digest: None,
            provenance: None,
            configuration: serde_json::Value::Null,
        })
        .collect();
    let required = DomainSet::only(["code-exec"]);
    let declared: std::collections::BTreeMap<String, Vec<String>> = [
        ("bash".to_string(), vec!["code-exec".to_string()]),
        ("deploy_to_prod".to_string(), vec!["code-exec".to_string()]),
    ]
    .into_iter()
    .collect();
    let sliced = intent::slice_capabilities(&admitted, &required, &declared);
    let names: Vec<&str> = sliced.iter().map(|c| c.name.as_str()).collect();
    // `grep` is in the orientation floor and survives by name; the unadmitted
    // `deploy_to_prod` is not conjured into existence by being declared.
    assert!(names.contains(&"bash"));
    assert!(!names.contains(&"deploy_to_prod"));
    assert!(sliced.len() <= admitted.len());
}

/// A revoked grant stops narrowing and does not leave a remembered widening
/// behind. Revocation is checked on read, so it lands at the next authority
/// check rather than the next session.
#[test]
fn a_revoked_grant_grants_nothing() {
    let now = chrono::Utc::now();
    let mut revoked = envelope(PermissionCeiling::WorkspaceWrite);
    revoked.revoked_at = Some(now);
    let authority = Authority {
        autonomy: Autonomy::Delegated,
        attendance: vak_intent::Attendance::Supervised,
        envelope: Some(revoked),
    };
    assert_eq!(
        authority.permission_ceiling(now),
        PermissionCeiling::FullAccess,
        "a revoked grant must impose no ceiling, and confer none either"
    );
    assert_eq!(authority.spend_limit_usd(now), None);
    // And delegation buys nothing once the grant is gone.
    assert_eq!(
        authority.approval_ceiling(Stakes::Reversible, now, true),
        ApprovalCeiling::Ask
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

/// Ladder narrowing takes a prefix of the frozen ladder and never empties it:
/// a narrowing must not become an outage.
#[test]
fn a_ladder_prefix_never_empties_the_ladder() {
    let ladder = vec!["primary", "second", "third"];
    assert_eq!(
        intent::limit_ladder(&ladder, Some(2)),
        vec!["primary", "second"]
    );
    assert_eq!(intent::limit_ladder(&ladder, Some(0)), ladder);
    assert_eq!(intent::limit_ladder(&ladder, None), ladder);
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
