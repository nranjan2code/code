//! A connected account's erasure survives a restore (plan M7a-h): a
//! backup taken while the account's data was readable brings its key
//! back, and the restore destroys it again.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_core::Core;
use vak_core::backup::Conflict;
use vak_core::erasure::Cause;
use vak_llm::types::ContentBlock;
use vak_session::SessionLog;
use vak_session::objects::{TenantObjects, account_scope};

fn results(ledger: &std::path::Path) -> String {
    SessionLog::open_read_only(ledger.to_path_buf())
        .unwrap()
        .chain_to_root()
        .iter()
        .filter_map(|entry| match &entry.payload {
            vak_session::EntryPayload::Message(record) => Some(record.message.content.clone()),
            _ => None,
        })
        .flatten()
        .filter_map(|block| match block {
            ContentBlock::ToolResult { content, .. } => Some(content),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn provider_account_erasure_reapplies_after_restore() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = Core::new_with_trust(work.path().to_path_buf(), true).unwrap();
    let account = "acct-restore";
    let mut log = core.start_session().await.unwrap();
    let ledger = log.path().to_path_buf();
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message::user_text("what did Dana say about the lease"),
        meta: None,
    })
    .unwrap();
    log.result_from_account("call-mail", account);
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message {
            role: vak_llm::Role::User,
            content: vec![
                ContentBlock::tool_result("call-mail", "From Dana: the lease is signed"),
                ContentBlock::tool_result("call-read", "notes: buy stamps"),
            ],
        },
        meta: None,
    })
    .unwrap();
    drop(log);
    assert!(results(&ledger).contains("lease is signed"));
    let tenant = TenantObjects::for_tenant(&vak_config::paths::local_tenant_home()).unwrap();
    let has_key = |tenant: &TenantObjects| {
        tenant
            .scopes_with_prefix(&account_scope(account))
            .unwrap()
            .contains(&account_scope(account))
    };

    let backup = tempfile::tempdir().unwrap();
    core.backup_create(backup.path(), false).unwrap();
    let receipt = core.erase_account(account, Cause::Person, None).unwrap();
    assert!(!has_key(&tenant));

    let report = core.backup_restore(backup.path(), Conflict::Skip).unwrap();
    assert_eq!(report.erasures_reapplied, 1);
    assert!(report.keys_removed >= 1, "{report:?}");

    assert!(!has_key(&tenant) && tenant.scope_destroyed(&account_scope(account)));
    let now = results(&ledger);
    assert!(!now.contains("lease") && !now.contains("Dana"), "{now}");
    assert!(now.contains(vak_session::log::ACCOUNT_REMOVED_TEXT));
    assert!(
        now.contains("buy stamps"),
        "the rest of the conversation stays"
    );
    assert_eq!(core.erasure_receipts(), [receipt]);
}
