//! M6 exit test `turn_path_reads_flat` (data-architecture plan M6.3): what
//! a turn reads to decide (whether a request was already admitted, the
//! routing evidence, a commitment's state) costs the same however long the
//! history is. Each read here runs after every sealed segment of its chain
//! has been made unreadable: a read that replayed history would fail or
//! answer wrongly.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

/// Incompressible text, so a few rows fill and seal a segment.
fn noise(bytes: usize) -> String {
    let mut out = String::new();
    while out.len() < bytes {
        out.push_str(&uuid::Uuid::now_v7().simple().to_string());
    }
    out
}

/// Makes every sealed segment of `chain` unreadable; returns how many.
fn ruin_sealed(chain: &Path) -> usize {
    let mut ruined = 0;
    for entry in std::fs::read_dir(chain).unwrap().flatten() {
        if entry.file_name().to_string_lossy().ends_with(".sealed") {
            std::fs::write(entry.path(), b"unreadable").unwrap();
            ruined += 1;
        }
    }
    ruined
}

#[test]
fn turn_path_reads_flat() {
    vak_config::paths::isolate_home_for_tests();
    let home = vak_config::paths::data_home().join("flat-agent");
    std::fs::create_dir_all(&home).unwrap();

    // ---- routing evidence ----
    let evidence = vak_core::routing::EvidenceLedger::new(&home);
    // Every other row names a leg of its own with incompressible text, so
    // the chain seals segments in a few hundred rows.
    for n in 0..200u64 {
        evidence
            .append(&vak_core::routing::EvidenceRow {
                ts: chrono::Utc::now(),
                provider: "p".into(),
                model: if n % 2 == 0 { noise(16384) } else { "m".into() },
                outcome: "success".into(),
                latency_ms: n,
                trace: None,
                actor: None,
            })
            .unwrap();
    }
    let before = evidence.snapshot();
    let chain = vak_config::scope::AgentScope::new(&home).routing_evidence();
    assert!(ruin_sealed(&chain) > 0, "the evidence crossed a seal");
    let after = evidence.snapshot();
    let leg = |snapshot: &vak_llm::EvidenceSnapshot, model: &str| {
        snapshot.by_key[&("p".to_string(), model.to_string())].success
    };
    assert_eq!(leg(&after, "m"), 100);
    assert_eq!(after.by_key.len(), 101);
    assert_eq!(leg(&before, "m"), leg(&after, "m"));

    // ---- commitments ----
    let ledger = vak_commit::CommitmentLedger::new(&home);
    let mut ids = Vec::new();
    for _ in 0..80 {
        let mut spec = vak_commit::spec_from_reading(
            noise(16384),
            vak_intent::Reading::default(),
            Vec::new(),
            std::path::PathBuf::from("/tmp"),
            Default::default(),
        );
        spec.objective = noise(16384);
        ids.push(ledger.open_commitment(spec).unwrap());
    }
    assert_eq!(ledger.all().len(), 80);
    assert!(
        ruin_sealed(ledger.path()) > 0,
        "the commitments crossed a seal"
    );
    // The first commitment's events all lie in a ruined segment; its state
    // still reads, and closing it is still checked against that state.
    let first = ledger.get(&ids[0]).unwrap().expect("still projected");
    assert!(!first.phase.is_terminal());
    ledger
        .append(&vak_commit::Event::new(
            ids[0].clone(),
            vak_commit::EventKind::Superseded {
                by: ids[1].clone(),
                reason: "flat".into(),
            },
        ))
        .unwrap();
    assert!(ledger.get(&ids[0]).unwrap().unwrap().phase.is_terminal());
    assert!(matches!(
        ledger.append(&vak_commit::Event::new(
            ids[0].clone(),
            vak_commit::EventKind::Superseded {
                by: ids[2].clone(),
                reason: "again".into(),
            },
        )),
        Err(vak_commit::LedgerError::AlreadyClosed(_))
    ));

    // ---- request admission ----
    let cwd = tempfile::tempdir().unwrap();
    vak_config::spaces::bind(cwd.path()).unwrap();
    let id = uuid::Uuid::now_v7().to_string();
    let path = vak_session::SessionPath::new_session_file(&home, cwd.path(), &id);
    let header: vak_session::SessionHeader = serde_json::from_value(serde_json::json!({
        "session_id": id,
        "created_at": chrono::Utc::now(),
        "cwd": cwd.path(),
        "contract": {
            "app_version": "test", "provider": "p", "model": "m",
            "route_ladder": [], "route_objective": "", "route_annotations": [],
            "system_prompt": "", "permission_mode": "read-only",
            "capabilities": [], "prompt_layers": []
        }
    }))
    .unwrap();
    let mut log = vak_session::SessionLog::create(path.clone(), header).unwrap();
    let mut data = std::collections::BTreeMap::new();
    data.insert("request_id".to_string(), "req-early".to_string());
    log.append_activity(vak_session::ActivityRecord {
        activity_id: "a1".into(),
        kind: vak_session::ActivityKind::Run,
        status: vak_session::ActivityStatus::Succeeded,
        label: "admitted".into(),
        detail: None,
        data,
    })
    .unwrap();
    // A long session seals its segments as it goes: the admission now
    // lies in a sealed segment, behind later history.
    for _ in 0..3 {
        for _ in 0..20 {
            log.append_message(vak_session::MessageRecord {
                message: vak_llm::Message::user_text(noise(1024)),
                meta: None,
            })
            .unwrap();
        }
        log.seal_segment().unwrap();
    }
    assert!(log.has_request_admission("req-early").unwrap());
    drop(log);
    assert!(ruin_sealed(&path) >= 3, "the session sealed its segments");
    // No handle holds the history and its sealed segments are unreadable:
    // the answer still comes from one ref.
    assert!(vak_session::log::request_admitted(&id, "req-early").unwrap());
    assert!(!vak_session::log::request_admitted(&id, "req-never").unwrap());
}
