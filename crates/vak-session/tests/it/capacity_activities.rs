#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Capacity activities are ledger entries but never model-visible
//! (docs/design/68-context-engine.md §1, §4): `SessionLog::latest_capacity_profile`
//! reconstructs the most recent profile for a key from `CapacityProbe` /
//! `CapacityFeedback` activities, and a session carrying them projects
//! exactly as it would without them.
//!
//! `CapacityProfile`/`ProfileKey` live in vak-agent (which depends on
//! vak-session, not the reverse), so this test stands in with small local
//! types that have the same JSON shape a real caller would serialize —
//! `latest_capacity_profile` only round-trips JSON, it never interprets it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tempfile::tempdir;

use vak_llm::Message;
use vak_session::SessionLog;
use vak_session::types::{
    ActivityKind, ActivityRecord, ActivityStatus, FrozenContract, MessageRecord, SessionHeader,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FakeKey {
    provider: String,
    model: String,
    quantisation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct FakeProfile {
    instruction_horizon_tokens: u64,
    needs_reprobe: bool,
}

fn header() -> SessionHeader {
    SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "s-capacity".into(),
        created_at: chrono::Utc::now(),
        cwd: PathBuf::from("/tmp/proj"),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0.1.0".into(),
            provider: "ollama".into(),
            model: "gemma4:e2b-mlx".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "system prompt v1".into(),
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    }
}

fn capacity_activity(
    kind: ActivityKind,
    key: &FakeKey,
    profile: &FakeProfile,
    id: &str,
) -> ActivityRecord {
    let mut data = std::collections::BTreeMap::new();
    data.insert("key".into(), serde_json::to_string(key).unwrap());
    data.insert("profile".into(), serde_json::to_string(profile).unwrap());
    ActivityRecord {
        activity_id: id.into(),
        kind,
        status: ActivityStatus::Succeeded,
        label: "capacity".into(),
        detail: None,
        data,
    }
}

#[test]
fn latest_capacity_profile_returns_the_most_recent_matching_entry() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("capacity.jsonl");
    let mut log = SessionLog::create(path, header()).unwrap();

    let key = FakeKey {
        provider: "ollama".into(),
        model: "gemma4:e2b-mlx".into(),
        quantisation: None,
    };
    let other_key = FakeKey {
        provider: "anthropic".into(),
        model: "claude-sonnet-5".into(),
        quantisation: None,
    };

    let probed = FakeProfile {
        instruction_horizon_tokens: 20_000,
        needs_reprobe: false,
    };
    let fed_back = FakeProfile {
        instruction_horizon_tokens: 18_000,
        needs_reprobe: true,
    };
    let unrelated = FakeProfile {
        instruction_horizon_tokens: 100_000,
        needs_reprobe: false,
    };

    log.append_activity(capacity_activity(
        ActivityKind::CapacityProbe,
        &key,
        &probed,
        "a1",
    ))
    .unwrap();
    log.append_activity(capacity_activity(
        ActivityKind::CapacityProbe,
        &other_key,
        &unrelated,
        "a2",
    ))
    .unwrap();
    log.append_activity(capacity_activity(
        ActivityKind::CapacityFeedback,
        &key,
        &fed_back,
        "a3",
    ))
    .unwrap();

    let latest: FakeProfile = log
        .latest_capacity_profile(&key)
        .expect("a profile must be found for the matching key");
    assert_eq!(
        latest, fed_back,
        "feedback activity must win over the earlier probe"
    );

    let latest_other: FakeProfile = log
        .latest_capacity_profile(&other_key)
        .expect("the other key's profile must be found independently");
    assert_eq!(latest_other, unrelated);

    let missing_key = FakeKey {
        provider: "openai".into(),
        model: "gpt-5.6".into(),
        quantisation: None,
    };
    assert!(
        log.latest_capacity_profile::<_, FakeProfile>(&missing_key)
            .is_none()
    );
}

#[test]
fn capacity_activities_never_change_the_model_visible_projection() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("derive_parity.jsonl");
    let mut log = SessionLog::create(path, header()).unwrap();
    log.append_message(MessageRecord {
        message: Message::user_text("what's the weather"),
        meta: None,
    })
    .unwrap();
    let before = log.derive_messages();

    let key = FakeKey {
        provider: "ollama".into(),
        model: "gemma4:e2b-mlx".into(),
        quantisation: None,
    };
    let profile = FakeProfile {
        instruction_horizon_tokens: 13_000,
        needs_reprobe: false,
    };
    log.append_activity(capacity_activity(
        ActivityKind::CapacityProbe,
        &key,
        &profile,
        "p1",
    ))
    .unwrap();
    log.append_activity(capacity_activity(
        ActivityKind::CapacityFeedback,
        &key,
        &profile,
        "p2",
    ))
    .unwrap();

    let after = log.derive_messages();
    assert_eq!(
        before, after,
        "capacity-probe/capacity-feedback activities must never reach the model"
    );
}
