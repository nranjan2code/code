#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde::Serialize;
use serde::de::DeserializeOwned;
use vak_session::ids::PrincipalId;
use vak_session::trace::Traced;

/// One durable record type, as the data dictionary describes it
/// (`docs/reference/records.md`).
struct Record {
    row_type: &'static str,
    type_name: &'static str,
    /// Source file of the struct, relative to the workspace root.
    source: &'static str,
    /// `ledger` is append-only JSONL; `record` is one file per entity.
    class: &'static str,
    stored_in: &'static str,
    version: u32,
}

/// A row written under an actor reads back naming that actor, and writes it
/// out again unchanged: the field is part of the row's schema, not ambient.
fn record<T: Traced + Serialize + DeserializeOwned>(
    type_name: &'static str,
    source: &'static str,
    class: &'static str,
    stored_in: &'static str,
    mut sample: serde_json::Value,
) -> Record {
    assert!(T::ROW_TYPE.len() > 2);
    let actor = PrincipalId::new();
    sample["actor"] = serde_json::json!(actor.to_string());
    let row: T =
        serde_json::from_value(sample).unwrap_or_else(|error| panic!("{}: {error}", T::ROW_TYPE));
    assert_eq!(row.actor(), Some(&actor), "{}", T::ROW_TYPE);
    let again = serde_json::to_value(&row).unwrap();
    assert_eq!(again["actor"], serde_json::json!(actor.to_string()));
    Record {
        row_type: T::ROW_TYPE,
        type_name,
        source,
        class,
        stored_in,
        version: 1,
    }
}

fn outbox_sample() -> serde_json::Value {
    let job = vak_delivery::DeliveryJob {
        job_id: "job-1".into(),
        target: "log:main".into(),
        kind: vak_delivery::DeliveryKind::Assistant,
        content: vak_delivery::DeliveryContent::Answer(vak_delivery::AnswerDraft::from_markdown(
            "hello",
        )),
        profile: vak_delivery::DeliveryProfile::plain("log"),
        skill_registry: None,
        trace: None,
        actor: None,
    };
    serde_json::to_value(vak_delivery::outbox::OutboxRecord {
        schema_version: 1,
        job,
        state: vak_delivery::outbox::OutboxState::Pending,
        attempts: 0,
        created_at_ms: 1,
        updated_at_ms: 1,
        packet: None,
        last_error: None,
        trace: None,
        actor: None,
    })
    .unwrap()
}

/// Every durable record type that carries the trace key and an actor, with a
/// sample row. Adding a ledger row type means adding it here; the count test
/// below fails until it is.
fn records() -> Vec<Record> {
    let ts = "2026-10-02T00:00:00Z";
    let plan = serde_json::json!({
        "id": "env-1", "outcome_revision": 1, "input_root": "/in", "task_root": "/task",
        "backend": "local", "image": null, "network_policy": "none", "setup_recipe": [],
    });
    vec![
        record::<vak_core::finops::CostRow>(
            "CostRow",
            "crates/vak-core/src/finops.rs",
            "ledger",
            "cost-log",
            serde_json::json!({
                "ts": ts, "model": "m", "input_tokens": 1, "output_tokens": 1,
                "source": "estimated", "session_id": "s",
            }),
        ),
        record::<vak_core::finops::ActivityRow>(
            "ActivityRow",
            "crates/vak-core/src/finops.rs",
            "ledger",
            "activity-log",
            serde_json::json!({"ts": ts, "kind": "tool", "name": "n", "success": true}),
        ),
        record::<vak_core::finops::BudgetAlertRow>(
            "BudgetAlertRow",
            "crates/vak-core/src/finops.rs",
            "ledger",
            "budget-alerts",
            serde_json::json!({
                "kind": "budget_alert", "ts": ts, "level": "eighty", "day_total_usd": 1.0,
                "session_id": "s",
            }),
        ),
        record::<vak_core::routing::EvidenceRow>(
            "EvidenceRow",
            "crates/vak-core/src/routing.rs",
            "ledger",
            "routing-evidence",
            serde_json::json!({
                "ts": ts, "provider": "p", "model": "m", "outcome": "success", "latency_ms": 1,
            }),
        ),
        record::<vak_core::misread::MisreadRow>(
            "MisreadRow",
            "crates/vak-core/src/misread.rs",
            "ledger",
            "intent-evidence",
            serde_json::json!({
                "ts": ts, "act": "a", "stakes": "s", "tier": "t", "resolver_version": 1,
                "outcome": "o",
            }),
        ),
        record::<vak_core::security_events::SecurityEvent>(
            "SecurityEvent",
            "crates/vak-core/src/security_events.rs",
            "ledger",
            "security-events",
            serde_json::json!({"ts": ts, "kind": "rate_limit", "label": "l", "detail": "d"}),
        ),
        record::<vak_core::inbox::Entry>(
            "Entry",
            "crates/vak-core/src/inbox.rs",
            "ledger",
            "inbox",
            serde_json::json!({"id": "i", "ts": ts, "kind": "heartbeat", "title": "t", "body": "b"}),
        ),
        record::<vak_commit::Event>(
            "Event",
            "crates/vak-commit/src/ledger.rs",
            "ledger",
            "commitments",
            serde_json::json!({
                "event_id": "e", "commitment_id": "c", "ts": ts, "kind": "episode-started",
                "episode_id": "x", "session_id": "s",
            }),
        ),
        record::<crate::operations::IncidentRecord>(
            "IncidentRecord",
            "crates/vak-server/src/operations.rs",
            "ledger",
            "operations/incidents",
            serde_json::json!({
                "id": "i", "fingerprint": "f", "severity": "s", "status": "open", "source": "x",
                "title": "t", "detail": "d", "first_seen": ts, "last_seen": ts,
                "occurrences": 1, "workspace": null, "evidence": [], "resolution": null,
            }),
        ),
        record::<crate::operations::ActionReceipt>(
            "ActionReceipt",
            "crates/vak-server/src/operations.rs",
            "ledger",
            "operations/actions",
            serde_json::json!({
                "receipt_id": "r", "service": "s", "action": "a", "requested_at": ts,
                "completed_at": ts, "succeeded": true, "persisted": true,
                "verification": {"status": "s", "before": "b", "after": "a", "detail": "d"},
            }),
        ),
        record::<crate::coworking::AudienceGrant>(
            "AudienceGrant",
            "crates/vak-server/src/coworking.rs",
            "ledger",
            "coworking/grants.jsonl",
            serde_json::json!({
                "grant_id": "g", "principal_id": "p", "display_name": "d",
                "conversation_id": "c", "audience_id": "a", "capabilities": [],
                "token_hash": "h", "created_at": ts, "expires_at": ts,
            }),
        ),
        record::<vak_delivery::outbox::OutboxRecord>(
            "OutboxRecord",
            "crates/vak-delivery/src/outbox.rs",
            "record",
            "gateway outbox, one file per delivery job",
            outbox_sample(),
        ),
        record::<vak_sandbox::EnvironmentRecord>(
            "EnvironmentRecord",
            "crates/vak-sandbox/src/lib.rs",
            "ledger",
            "sandbox/records.jsonl",
            serde_json::json!({
                "record_id": "r", "environment_id": "env-1", "state": "Ready", "plan": plan,
                "updated_at": ts,
            }),
        ),
        record::<vak_sandbox::PreviewPreparationRecord>(
            "PreviewPreparationRecord",
            "crates/vak-sandbox/src/lib.rs",
            "ledger",
            "sandbox/records.jsonl",
            serde_json::json!({
                "record_id": "r", "session_id": "s", "result_id": "x", "candidate_id": "c",
                "candidate_digest": "d", "environment_id": "e", "state": "Ready",
                "command": "c", "evidence": "e", "updated_at": ts,
            }),
        ),
        record::<vak_sandbox::PromotionRecord>(
            "PromotionRecord",
            "crates/vak-sandbox/src/lib.rs",
            "ledger",
            "sandbox/records.jsonl",
            serde_json::json!({
                "record_id": "r", "session_id": "s", "result_id": "x",
                "candidate_digest": "d", "candidate_id": "c", "updated_at": ts,
                "receipt": {
                    "candidate_id": "c", "applied": [], "before_hashes": [],
                    "after_hashes": [], "verification": [],
                },
            }),
        ),
        record::<vak_sandbox::CandidateRecord>(
            "CandidateRecord",
            "crates/vak-sandbox/src/lib.rs",
            "ledger",
            "sandbox/records.jsonl",
            serde_json::json!({
                "record_id": "r", "session_id": "s", "turn_id": "t", "result_id": "x",
                "execution_id": "e", "environment_id": "env", "candidate_digest": "d",
                "verified": true, "updated_at": ts,
                "candidate": {
                    "candidate_id": "c", "source_root": "/s", "destination_root": "/d",
                    "files": [],
                },
            }),
        ),
    ]
}

fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Every durable ledger row type carries the optional trace key and actor
/// (docs/design/73 §4), and `records()` names every type that does.
#[test]
fn every_ledger_row_type_is_traced() {
    let records = records();
    let mut names: Vec<&str> = records.iter().map(|r| r.row_type).collect();
    names.sort_unstable();
    let total = names.len();
    names.dedup();
    assert_eq!(total, names.len(), "row type names are unique");
    assert_eq!(total, 16);

    let mut declared = 0;
    for entry in std::fs::read_dir(workspace_root().join("crates"))
        .unwrap()
        .flatten()
    {
        let mut stack = vec![entry.path().join("src")];
        while let Some(dir) = stack.pop() {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            for item in read.flatten() {
                let path = item.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap_or_default();
                    declared += text
                        .lines()
                        .filter(|line| line.trim_start().starts_with("vak_session::impl_traced!("))
                        .count();
                }
            }
        }
    }
    assert_eq!(
        declared, total,
        "a type implements Traced that records() does not list"
    );
}

/// Each of the sixteen row types reads back an actor written onto its row
/// and writes it out again (the assertion lives in `record`).
#[test]
fn every_ledger_row_type_names_its_actor() {
    assert_eq!(records().len(), 16);
}

struct Field {
    name: String,
    ty: String,
    required: bool,
    note: String,
}

/// The fields of `pub struct <name>` in `source`, read from the declaration
/// so the dictionary cannot describe a type the code does not have.
fn struct_fields(source: &str, name: &str) -> Vec<Field> {
    let header = format!("pub struct {name} {{");
    let mut lines = source.lines();
    assert!(
        lines.any(|line| line.trim_end() == header),
        "struct {name} not found"
    );
    let mut fields = Vec::new();
    let mut attrs: Vec<String> = Vec::new();
    let mut docs: Vec<String> = Vec::new();
    let mut pending: Option<String> = None;
    for line in lines {
        let trimmed = line.trim();
        if let Some(open) = pending.as_mut() {
            open.push(' ');
            open.push_str(trimmed);
            let balanced = open.matches('<').count() == open.matches('>').count();
            if trimmed.ends_with(',') && balanced {
                let done = pending.take().unwrap();
                fields.push(field_from(&done, &attrs, &docs));
                attrs.clear();
                docs.clear();
            }
            continue;
        }
        if line == "}" {
            break;
        }
        if let Some(doc) = trimmed.strip_prefix("///") {
            docs.push(doc.trim().to_string());
        } else if trimmed.starts_with("#[") {
            attrs.push(trimmed.to_string());
        } else if let Some(rest) = trimmed.strip_prefix("pub ") {
            let balanced = rest.matches('<').count() == rest.matches('>').count();
            if trimmed.ends_with(',') && balanced {
                fields.push(field_from(rest, &attrs, &docs));
                attrs.clear();
                docs.clear();
            } else {
                pending = Some(rest.to_string());
            }
        }
    }
    fields
}

fn field_from(decl: &str, attrs: &[String], docs: &[String]) -> Field {
    let (name, ty) = decl.split_once(':').unwrap();
    let ty = ty.trim().trim_end_matches(',').trim().to_string();
    let attr_text = attrs.join(" ");
    let optional = ty.starts_with("Option<") || attr_text.contains("default");
    let mut note = docs.first().cloned().unwrap_or_default();
    if attr_text.contains("flatten") {
        note = format!("{note} (flattened into the row)")
            .trim()
            .to_string();
    }
    if let Some(rename) = attr_text.split("rename = \"").nth(1) {
        let wire = rename.split('"').next().unwrap_or_default();
        note = format!("{note} (written as `{wire}`)").trim().to_string();
    }
    Field {
        name: name.trim().to_string(),
        ty,
        required: !optional,
        note: note.replace('|', "\\|"),
    }
}

fn render_records_reference() -> String {
    let root = workspace_root();
    let mut out = String::from(
        "# Record reference\n\n\
         Generated from the record types by `records_reference_is_current`\n\
         (`crates/vak-server/src/traced_rows_tests.rs`). Do not edit by hand:\n\
         change the type, then run\n\
         `VAK_UPDATE_RECORDS=1 cargo test -p vak-server records_reference_is_current`.\n\n\
         Every record below carries an optional `trace` (the run's trace key) and\n\
         an optional `actor` (the principal that acted), both additive and omitted\n\
         when unknown (docs/design/73 section 4). Within a major version a field is\n\
         added, never removed, renamed or retyped, and readers ignore unknown\n\
         fields. A `ledger` is append-only JSONL; a `record` is one file per entity.\n",
    );
    let mut records = records();
    records.sort_by_key(|r| r.row_type);
    for record in &records {
        let source = std::fs::read_to_string(root.join(record.source)).unwrap();
        out.push_str(&format!(
            "\n## {}\n\nType `{}` in `{}`. Class: {}. Stored in: `{}`. Version: {}.\n\n\
             | Field | Type | Required | Notes |\n|---|---|---|---|\n",
            record.row_type,
            record.type_name,
            record.source,
            record.class,
            record.stored_in,
            record.version,
        ));
        for field in struct_fields(&source, record.type_name) {
            out.push_str(&format!(
                "| `{}` | `{}` | {} | {} |\n",
                field.name,
                field.ty.replace('|', "\\|"),
                if field.required { "yes" } else { "no" },
                field.note,
            ));
        }
    }
    out
}

/// The data dictionary matches the record types. Regenerate it with
/// `VAK_UPDATE_RECORDS=1 cargo test -p vak-server records_reference_is_current`.
#[test]
fn records_reference_is_current() {
    let path = workspace_root().join("docs/reference/records.md");
    let expected = render_records_reference();
    if std::env::var_os("VAK_UPDATE_RECORDS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &expected).unwrap();
    }
    let actual = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        actual, expected,
        "docs/reference/records.md is stale; run `VAK_UPDATE_RECORDS=1 cargo test -p vak-server records_reference_is_current`"
    );
}

#[test]
fn a_row_without_a_trace_omits_the_fields_and_ignores_unknown_ones() {
    let dir = tempfile::tempdir().unwrap();
    let event = vak_core::security_events::record(
        &vak_config::scope::AgentScope::new(dir.path()),
        vak_core::security_events::EventKind::RateLimit,
        "l",
        "d",
        None,
    );
    assert!(event.trace().is_none() && event.actor().is_none());
    let json = serde_json::to_string(&event).unwrap();
    assert!(!json.contains("trace") && !json.contains("actor"), "{json}");
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["unknown_future_field"] = serde_json::json!(1);
    let back: vak_core::security_events::SecurityEvent = serde_json::from_value(value).unwrap();
    assert_eq!(back, event);
}
