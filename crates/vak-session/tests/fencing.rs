//! Fencing (plan M4.1): once the tenant store is restored after a process
//! opened it, that process writes nothing more. Its own test binary,
//! because being fenced is process-wide and sticky.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use vak_session::chain::RecordChain;
use vak_session::objects::{Objects, TenantObjects};
use vak_session::types::{FrozenContract, SessionError, SessionHeader};
use vak_session::{SessionLog, SessionPath, fence};

fn header(id: &str, cwd: &Path) -> SessionHeader {
    SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: id.to_string(),
        created_at: chrono::Utc::now(),
        cwd: cwd.to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "test".into(),
            provider: "scripted".into(),
            model: "m".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: String::new(),
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    }
}

fn is_fenced<T>(result: Result<T, SessionError>) -> bool {
    matches!(result, Err(SessionError::Fenced { .. }))
}

#[test]
fn restore_fences_old_writer() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().canonicalize().unwrap();
    vak_config::spaces::bind(&cwd).unwrap();
    let data = vak_config::paths::data_home();
    let tenant_home = vak_config::paths::local_tenant_home();
    let agent_home = vak_config::paths::agent_home_at(&data, "vak");
    let chain = RecordChain::at(data.join("fencing-test"));
    let document = data.join("agents/vak/notes/fencing.md");

    // Before the restore this process writes everywhere.
    let tenant = TenantObjects::for_tenant(&tenant_home).unwrap();
    let before = tenant.writer_epoch();
    let first = uuid::Uuid::now_v7().to_string();
    let mut open_log = SessionLog::create(
        SessionPath::new_session_file(&agent_home, &cwd, &first),
        header(&first, &cwd),
    )
    .unwrap();
    open_log
        .begin_turn(&uuid::Uuid::now_v7().to_string())
        .unwrap();
    chain.append(&serde_json::json!({"row": 1})).unwrap();
    vak_session::documents::create(&document, "kept").unwrap();
    tenant.put(b"payload", "conversation:x").unwrap();
    let process = fence::process();
    fence::renew_liveness(
        &tenant_home,
        chrono::Utc::now() + chrono::Duration::seconds(60),
    )
    .unwrap();
    assert!(fence::is_alive(&tenant_home, &process, chrono::Utc::now()).unwrap());
    assert!(!fence::is_fenced());

    // Another writer restores the store: its epoch moves past ours.
    let restored = tenant.store().restore().unwrap();
    assert!(restored > before.0);

    assert!(fence::is_fenced());
    assert!(is_fenced(fence::check()));
    // No new turn on a ledger opened before the restore.
    assert!(is_fenced(
        open_log.begin_turn(&uuid::Uuid::now_v7().to_string())
    ));
    drop(open_log);
    // No ledger opened or created for writing.
    assert!(is_fenced(SessionLog::open(
        SessionPath::existing_session_file(&agent_home, &cwd, &first,)
    )));
    let second = uuid::Uuid::now_v7().to_string();
    assert!(is_fenced(SessionLog::create(
        SessionPath::new_session_file(&agent_home, &cwd, &second),
        header(&second, &cwd),
    )));
    // No side-ledger row, no object, no Document, no liveness.
    assert!(is_fenced(chain.append(&serde_json::json!({"row": 2}))));
    assert!(is_fenced(tenant.put(b"other", "conversation:x")));
    let refused =
        vak_session::documents::update(&document, |_| Ok(Some(("changed".to_string(), ()))))
            .unwrap_err();
    assert!(refused.contains("fenced"), "{refused}");
    assert!(vak_session::documents::create(&data.join("agents/vak/notes/new.md"), "x").is_err());
    assert!(is_fenced(fence::renew_liveness(
        &tenant_home,
        chrono::Utc::now() + chrono::Duration::seconds(60),
    )));
    // What was written before the restore still reads.
    assert_eq!(
        vak_session::documents::read(&document).unwrap().as_deref(),
        Some("kept")
    );
    assert_eq!(chain.read::<serde_json::Value>().len(), 1);
}
