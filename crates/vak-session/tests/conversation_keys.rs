//! A conversation's ledger is sealed under its own key (plan M7a-a).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use vak_llm::Message;
use vak_session::SessionLog;
use vak_session::objects::{TenantObjects, conversation_scope};
use vak_session::types::{FrozenContract, MessageRecord, SessionError, SessionHeader};

const SECRET: &str = "the launch is on the fourteenth";

fn header(session_id: &str) -> SessionHeader {
    SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: session_id.into(),
        created_at: chrono::Utc::now(),
        cwd: PathBuf::from("/tmp/proj"),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0.1.0".into(),
            provider: "anthropic".into(),
            model: "m".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "an identity only this conversation was given".into(),
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    }
}

fn written(dir: &Path, session_id: &str) -> PathBuf {
    let path = dir.join("ledger");
    let mut log = SessionLog::create(path.clone(), header(session_id)).unwrap();
    log.append_message(MessageRecord {
        message: Message::user_text(SECRET),
        meta: None,
    })
    .unwrap();
    path
}

fn stored_bytes(path: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    SessionLog::segment_files(path)
        .into_iter()
        .map(|segment| {
            let bytes = std::fs::read(&segment).unwrap();
            (segment, bytes)
        })
        .collect()
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[test]
fn a_ledger_holds_no_plaintext_and_reads_back_through_its_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = written(dir.path(), "ses-keys-sealed");

    for (segment, bytes) in stored_bytes(&path) {
        let report = vak_storage::records::verify_chain(&segment).unwrap();
        assert_eq!(
            report.encrypted_frames, report.entries,
            "every frame is sealed"
        );
        assert!(report.entries >= 2);
        // zstd alone would leave short strings findable; the seal does not.
        assert!(!contains(&bytes, "launch"));
        assert!(!contains(&bytes, "ses-keys-sealed"));
        assert!(!contains(&bytes, "identity"));
    }

    assert_eq!(
        vak_session::keys::scope_of(&path).as_deref(),
        Some(conversation_scope("ses-keys-sealed").as_str())
    );
    assert_eq!(
        SessionLog::read_header(&path).unwrap().session_id,
        "ses-keys-sealed"
    );
    assert!(SessionLog::text(&path).contains(SECRET));
    let reopened = SessionLog::open(path.clone()).unwrap();
    assert_eq!(reopened.len(), 2);
    drop(reopened);
    let mut seen = 0;
    vak_session::tail::tail(&path, Default::default(), |_, frame| {
        seen += 1;
        assert!(!frame.is_empty());
        true
    });
    assert_eq!(seen, 2, "a derived index reads the ledger through its key");
}

#[test]
fn a_destroyed_key_leaves_the_bytes_and_the_chain_and_nothing_readable() {
    let dir = tempfile::tempdir().unwrap();
    let path = written(dir.path(), "ses-keys-destroyed");
    let before = stored_bytes(&path);
    // The index recorded where the directive is while the key was alive.
    let tenant = TenantObjects::for_tenant(&vak_config::paths::local_tenant_home()).unwrap();

    tenant
        .destroy_scope_key(&conversation_scope("ses-keys-destroyed"))
        .unwrap();

    assert_eq!(
        stored_bytes(&path),
        before,
        "erasing changes no ledger byte"
    );
    for (segment, _) in &before {
        let report = vak_storage::records::verify_chain(segment).unwrap();
        assert_eq!(report.entries, 2, "the chain still verifies with no key");
        assert!(report.torn_tail.is_none());
    }
    assert!(matches!(
        SessionLog::read_header(&path),
        Err(SessionError::Erased(_))
    ));
    assert!(matches!(
        SessionLog::open_read_only(path.clone()),
        Err(SessionError::Erased(_))
    ));
    assert!(matches!(
        SessionLog::open(path.clone()),
        Err(SessionError::Erased(_))
    ));
    assert_eq!(SessionLog::text(&path), "");
    assert_eq!(SessionLog::scan(&path, |_| true), 0);
    let mut seen = 0;
    vak_session::tail::tail(&path, Default::default(), |_, _| {
        seen += 1;
        true
    });
    assert_eq!(seen, 0);
    assert!(
        tenant
            .create_scope_key(&conversation_scope("ses-keys-destroyed"))
            .is_err(),
        "a destroyed conversation's key is never made again"
    );
}

#[test]
fn destroying_one_conversation_leaves_another_readable() {
    let dir = tempfile::tempdir().unwrap();
    let kept = written(&dir.path().join("kept"), "ses-keys-kept");
    let gone = written(&dir.path().join("gone"), "ses-keys-gone");
    TenantObjects::for_tenant(&vak_config::paths::local_tenant_home())
        .unwrap()
        .destroy_scope_key(&conversation_scope("ses-keys-gone"))
        .unwrap();
    assert!(SessionLog::read_header(&gone).is_err());
    assert!(SessionLog::text(&kept).contains(SECRET));
}

#[test]
fn a_ledger_with_no_key_is_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = written(dir.path(), "ses-keys-undeclared");
    std::fs::remove_file(path.join("KEY")).unwrap();
    assert!(matches!(
        SessionLog::read_header(&path),
        Err(SessionError::Unencrypted(_))
    ));
    assert!(matches!(
        SessionLog::open(path.clone()),
        Err(SessionError::Unencrypted(_))
    ));
    assert_eq!(SessionLog::text(&path), "");
}
