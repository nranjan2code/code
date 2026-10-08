//! Erasing a conversation (plan M7a-e, docs/design/74 §4): the key goes,
//! the ledger's bytes stay, what was derived in plaintext is removed, and
//! a signed receipt says so.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use vak_core::erasure::{Cause, ErasureError};
use vak_core::{Core, trash};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::SessionLog;

struct Answers {
    capacity_key: crate::support::CapacityKey,
}

#[async_trait::async_trait]
impl Provider for Answers {
    fn name(&self) -> &str {
        "scripted"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.0.clone()
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let m = AssistantMessage {
            content: vec![ContentBlock::text("Understood, the quillfeather plan.")],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            model: "test-model".into(),
            response_id: None,
        };
        let (mut sink, rx) = stream::channel(64);
        sink.push(stream::StreamEvent::Start { partial: m.clone() });
        sink.close_message(m).await;
        Ok(rx)
    }
}

/// These tests share one home and so one catalog, which two of them
/// rebuild: each runs alone, or one would read the index mid-rebuild.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn core(cwd: &std::path::Path) -> Core {
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        "[memory]\nreflection = false\n\n[stop_policy]\nenabled = false\n",
    )
    .unwrap();
    let core = Core::new_with_trust(cwd.to_path_buf(), true).unwrap();
    core.set_provider_instance(Arc::new(Answers {
        capacity_key: crate::support::CapacityKey::default(),
    }));
    core
}

/// A conversation with one turn about `secret`; its id and ledger.
async fn conversation(core: &Core, secret: &str) -> (String, std::path::PathBuf) {
    let session = core.start_session().await.unwrap();
    let id = session.header().unwrap().session_id.clone();
    let ledger = session.path().to_path_buf();
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let (_, session) = core
        .run_turn_with(
            session,
            &format!("keep in mind: {secret}"),
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await
        .unwrap();
    drop(session);
    (id, ledger)
}

fn bytes_of(ledger: &std::path::Path) -> Vec<Vec<u8>> {
    SessionLog::segment_files(ledger)
        .iter()
        .map(|segment| std::fs::read(segment).unwrap())
        .collect()
}

fn indexed(core: &Core, needle: &str) -> bool {
    let catalog = core.catalog().unwrap();
    catalog.catch_up().unwrap();
    catalog
        .dump()
        .unwrap()
        .iter()
        .any(|row| row.contains(needle))
}

#[tokio::test]
async fn erasure_follows_lineage() {
    let _serial = SERIAL.lock().await;
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = core(work.path());
    let shared = core.shared_scope();
    let (gone, gone_ledger) = conversation(&core, "the zephyrine launch date").await;
    let (kept, kept_ledger) = conversation(&core, "the marigold budget").await;
    // What was derived from the conversation in plaintext: a memory note,
    // an inbox entry, and its rows in the search index.
    let home = shared.agents_dir().join("vak");
    vak_core::memory::append_note(&home, work.path(), "fact", "", &gone, "launch is zephyrine")
        .unwrap();
    vak_core::memory::append_note(&home, work.path(), "fact", "", &kept, "budget is marigold")
        .unwrap();
    let agent = vak_config::scope::AgentScope::new(&home);
    vak_core::inbox::record(
        &agent,
        vak_core::inbox::Kind::TaskSummary,
        "zephyrine summary",
        "the launch moved",
        Some(&gone),
        None,
        None,
    )
    .unwrap();
    assert!(indexed(&core, "zephyrine"));
    let before = bytes_of(&gone_ledger);

    // A conversation is erased from the trash, never straight from use.
    assert!(matches!(core.erasure_preview(&gone), Ok(preview) if preview.trashed_at.is_none()));
    assert!(matches!(
        core.erase_conversation(&gone, None, Cause::Person, None),
        Err(ErasureError::NotInTrash)
    ));
    trash::set(&shared, std::slice::from_ref(&gone), true).unwrap();
    let preview = core.erasure_preview(&gone).unwrap();
    assert_eq!(preview.conversations, std::slice::from_ref(&gone));
    assert_eq!(preview.memory_notes, 1);

    let receipt = core
        .erase_conversation(&gone, Some(&preview.digest), Cause::Person, None)
        .unwrap();

    // erasure_leaves_ledger_bytes_unchanged: the frames are where they
    // were, the chain verifies, and nothing opens them.
    assert_eq!(bytes_of(&gone_ledger), before);
    for segment in SessionLog::segment_files(&gone_ledger) {
        assert!(
            vak_storage::records::verify_chain(&segment)
                .unwrap()
                .entries
                > 1
        );
    }
    assert!(matches!(
        SessionLog::read_header(&gone_ledger),
        Err(vak_session::SessionError::Erased(_))
    ));
    assert_eq!(SessionLog::text(&gone_ledger), "");

    // Its derived plaintext is gone, and the other conversation's is not.
    assert!(!indexed(&core, "zephyrine"));
    assert!(indexed(&core, "marigold"));
    let notes: Vec<String> = vak_core::memory::list_notes(&home, work.path())
        .into_iter()
        .map(|note| note.text)
        .collect();
    assert_eq!(notes, ["budget is marigold"]);
    assert!(vak_core::inbox::list(&agent, 10).is_empty());
    assert!(SessionLog::text(&kept_ledger).contains("marigold"));
    assert!(trash::is_trashed(&shared, &gone) && !trash::is_trashed(&shared, &kept));

    // The receipt: ids and counts, signed, and it says what it did not reach.
    assert_eq!(receipt.subject, gone);
    assert_eq!(receipt.cause, Cause::Person);
    assert_eq!(receipt.conversations, 1);
    assert_eq!(receipt.memory_notes_removed, 1);
    assert!(receipt.keys_destroyed >= 1 && receipt.search_rows_removed >= 1);
    assert!(!receipt.not_reached.is_empty());
    assert!(receipt.verifies());
    let mut forged = receipt.clone();
    forged.conversations = 0;
    assert!(!forged.verifies(), "a changed receipt no longer verifies");
    assert!(
        core.erasure_receipts().contains(&receipt),
        "the receipt is kept"
    );
    assert!(
        !serde_json::to_string(&core.erasure_receipts())
            .unwrap()
            .contains("zephyrine")
    );

    // The catalog says it was erased, traced to the receipt, and says the
    // same after it is rebuilt from the records. It holds no content.
    let catalog = core.catalog().unwrap();
    catalog.catch_up().unwrap();
    let traced = |catalog: &vak_catalog::Catalog| {
        let tombstone = catalog.open_node(&gone).unwrap().expect("a tombstone");
        assert_eq!(
            (tombstone.kind.as_str(), tombstone.status.as_deref()),
            ("erased", Some("erased"))
        );
        assert!(tombstone.title.is_none() && tombstone.locator.is_none());
        let lineage = catalog.lineage(&gone).unwrap().unwrap();
        assert!(
            lineage
                .path
                .iter()
                .any(|node| node.kind == "erasure" && node.id == receipt.id),
            "{:?}",
            lineage.path
        );
        assert!(catalog.session_dir(&gone).unwrap().is_none());
    };
    traced(&catalog);
    catalog.rebuild().unwrap();
    traced(&catalog);
    assert!(!indexed(&core, "zephyrine"));

    // Erased is final.
    assert!(matches!(
        core.erase_conversation(&gone, None, Cause::Person, None),
        Err(ErasureError::AlreadyErased)
    ));
    trash::set(&shared, std::slice::from_ref(&gone), false).unwrap();
    assert!(
        trash::is_trashed(&shared, &gone),
        "an erased conversation is not restored"
    );
}

#[tokio::test]
async fn stale_preview_cannot_authorise() {
    let _serial = SERIAL.lock().await;
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = core(work.path());
    let shared = core.shared_scope();
    let (id, ledger) = conversation(&core, "the heliotrope merger").await;
    trash::set(&shared, std::slice::from_ref(&id), true).unwrap();
    let preview = core.erasure_preview(&id).unwrap();

    // Something is derived from it after the person looked.
    let home = shared.agents_dir().join("vak");
    vak_core::memory::append_note(&home, work.path(), "fact", "", &id, "merger is heliotrope")
        .unwrap();
    assert!(matches!(
        core.erase_conversation(&id, Some(&preview.digest), Cause::Person, None),
        Err(ErasureError::StalePreview)
    ));
    assert!(
        SessionLog::read_header(&ledger).is_ok(),
        "nothing was destroyed"
    );

    // hold_blocks_every_destructive_transition: a held conversation is
    // erased neither by a person nor by the end of its trash window.
    core.hold_conversation(&id, true).unwrap();
    let fresh = core.erasure_preview(&id).unwrap();
    assert!(fresh.held);
    assert!(matches!(
        core.erase_conversation(&id, Some(&fresh.digest), Cause::Person, None),
        Err(ErasureError::Held)
    ));
    assert!(matches!(
        core.erase_conversation(&id, None, Cause::Policy, None),
        Err(ErasureError::Held)
    ));
    let plan = core.lifecycle_plan();
    assert!(
        plan.guarded
            .iter()
            .chain(std::iter::empty())
            .all(|kept| kept.item != id || kept.guard == vak_lifecycle::Guard::Held)
    );
    assert!(!plan.actions.iter().any(|action| action.item == id));
    assert!(SessionLog::read_header(&ledger).is_ok());

    // Released, the same fresh look authorises it.
    core.hold_conversation(&id, false).unwrap();
    let fresh = core.erasure_preview(&id).unwrap();
    core.erase_conversation(&id, Some(&fresh.digest), Cause::Person, None)
        .unwrap();
    assert!(SessionLog::read_header(&ledger).is_err());
}

fn transcript_of(ledger: &std::path::Path) -> String {
    SessionLog::open_read_only(ledger.to_path_buf())
        .unwrap()
        .derive_transcript()
        .iter()
        .map(|message| message.message.text_content())
        .collect::<Vec<_>>()
        .join("\n")
}

fn from_guest(principal: &str, name: &str, text: &str) -> vak_session::MessageRecord {
    vak_session::MessageRecord {
        message: vak_llm::Message::user_text(format!("{name}: {text}")),
        meta: Some(vak_session::MessageMeta {
            author_id: Some(principal.into()),
            author_name: Some(name.into()),
            ..Default::default()
        }),
    }
}

/// The owner erases what one guest wrote (plan M7a-f): that guest's words
/// leave the conversation and search, the other guest's and the owner's
/// stay, no ledger byte changes, and a signed receipt says so.
#[tokio::test]
async fn guest_erasure_keeps_owner_conversation() {
    let _serial = SERIAL.lock().await;
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = core(work.path());
    let (id, ledger) = conversation(&core, "the quince ledger").await;
    let mut log = SessionLog::open(ledger.clone()).unwrap();
    log.append_message(from_guest(
        "guest:asha",
        "Asha",
        "the venue holds ninety zebras",
    ))
    .unwrap();
    log.append_message(from_guest(
        "guest:ben",
        "Ben",
        "the caterer brings marzipan",
    ))
    .unwrap();
    drop(log);
    let before = bytes_of(&ledger);
    assert!(indexed(&core, "zebras") && indexed(&core, "marzipan"));
    assert_eq!(core.conversation_guests(&id), ["guest:asha", "guest:ben"]);

    // A guest who wrote nothing has nothing to erase.
    assert!(matches!(
        core.guest_erasure_preview(&id, "guest:nobody"),
        Err(ErasureError::NotFound(_))
    ));
    let preview = core.guest_erasure_preview(&id, "guest:asha").unwrap();
    assert!(preview.messages && preview.artifacts.is_empty() && !preview.held);
    assert!(matches!(
        core.erase_guest(
            &id,
            "guest:asha",
            Some("not the digest"),
            Cause::Person,
            None
        ),
        Err(ErasureError::StalePreview)
    ));
    core.hold_conversation(&id, true).unwrap();
    assert!(matches!(
        core.erase_guest(
            &id,
            "guest:asha",
            Some(&preview.digest),
            Cause::Person,
            None
        ),
        Err(ErasureError::Held)
    ));
    core.hold_conversation(&id, false).unwrap();

    let receipt = core
        .erase_guest(
            &id,
            "guest:asha",
            Some(&preview.digest),
            Cause::Person,
            None,
        )
        .unwrap();
    assert_eq!(
        (
            receipt.scope.as_str(),
            receipt.keys_destroyed,
            receipt.conversations
        ),
        ("guest", 1, 0)
    );
    assert_eq!(receipt.subject, format!("{id}/guest:asha"));
    assert!(receipt.verifies() && !receipt.not_reached.is_empty());

    assert_eq!(bytes_of(&ledger), before, "no ledger byte changes");
    let said = transcript_of(&ledger);
    assert!(!said.contains("zebras") && !said.contains("Asha"));
    assert!(said.contains(vak_session::log::REMOVED_TEXT));
    assert!(said.contains("marzipan") && said.contains("quince"));
    assert!(
        !indexed(&core, "zebras"),
        "search no longer holds their words"
    );
    assert!(indexed(&core, "marzipan") && indexed(&core, "quince"));
    assert!(!trash::is_trashed(&core.shared_scope(), &id));
    assert_eq!(core.conversation_guests(&id), ["guest:ben"]);
    assert!(matches!(
        core.erase_guest(&id, "guest:asha", None, Cause::Person, None),
        Err(ErasureError::NotFound(_))
    ));
}

/// Erasing a connected account (plan M7a-f) removes what it returned from
/// every conversation of every Agent that read it, and nothing else.
#[tokio::test]
async fn provider_account_erasure_spans_agents_and_conversations() {
    let _serial = SERIAL.lock().await;
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = core(work.path());
    let (first, first_ledger) = conversation(&core, "the walnut budget").await;
    let (_, second_ledger) = conversation(&core, "the hazel budget").await;
    // A conversation of another Agent, in that Agent's own home.
    let mut header = SessionLog::read_header(&first_ledger).unwrap();
    header.session_id = format!("{first}-scout");
    let scout_ledger =
        vak_config::scope::AgentScope::new(core.shared_scope().agents_dir().join("scout"))
            .session_file(work.path(), &header.session_id);
    std::fs::create_dir_all(scout_ledger.parent().unwrap()).unwrap();
    drop(SessionLog::create(scout_ledger.clone(), header).unwrap());

    let account = "acct-erasure-spans";
    for (ledger, mail, file) in [
        (
            &first_ledger,
            "From Dana: the lease is signed",
            "notes: buy stamps",
        ),
        (
            &second_ledger,
            "From Dana: the lease starts in May",
            "notes: call Omar",
        ),
        (
            &scout_ledger,
            "Calendar: lease walkthrough Tuesday",
            "notes: bring keys",
        ),
    ] {
        let mut log = SessionLog::open(ledger.clone()).unwrap();
        log.result_from_account("call-mail", account);
        log.append_message(vak_session::MessageRecord {
            message: vak_llm::Message {
                role: vak_llm::Role::User,
                content: vec![
                    ContentBlock::tool_result("call-mail", mail),
                    ContentBlock::tool_result("call-read", file),
                ],
            },
            meta: None,
        })
        .unwrap();
    }
    let ledgers = [&first_ledger, &second_ledger, &scout_ledger];
    let before: Vec<_> = ledgers.iter().map(|ledger| bytes_of(ledger)).collect();
    let results = |ledger: &std::path::Path| -> String {
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
    };
    assert!(
        ledgers
            .iter()
            .all(|ledger| results(ledger).contains("lease"))
    );
    assert!(
        indexed(&core, "lease"),
        "search held what the account returned"
    );

    assert!(matches!(
        core.erase_account("acct-never-read", Cause::Person, None),
        Err(ErasureError::NotFound(_))
    ));
    let receipt = core.erase_account(account, Cause::Person, None).unwrap();
    assert_eq!(
        (
            receipt.scope.as_str(),
            receipt.subject.as_str(),
            receipt.keys_destroyed
        ),
        ("account", account, 1)
    );
    assert!(receipt.verifies());
    assert!(
        receipt
            .not_reached
            .iter()
            .any(|line| line.contains("wrote from")),
        "the receipt says what the Agent wrote from it stays"
    );

    for (ledger, was) in ledgers.iter().zip(&before) {
        assert_eq!(&bytes_of(ledger), was, "no ledger byte changes");
        let now = results(ledger);
        assert!(!now.contains("lease") && !now.contains("Dana"), "{now}");
        assert!(now.contains(vak_session::log::ACCOUNT_REMOVED_TEXT));
        assert!(now.contains("notes:"), "other results are untouched");
    }
    assert!(transcript_of(&first_ledger).contains("walnut"));
    assert!(!indexed(&core, "lease"), "search no longer holds its data");
    assert!(indexed(&core, "walnut") && indexed(&core, "hazel"));
    assert!(matches!(
        core.erase_account(account, Cause::Person, None),
        Err(ErasureError::AlreadyErased)
    ));
}
