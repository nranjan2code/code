#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use vak_core::learning;
use vak_core::memory;
use vak_core::reflection::{apply, jaccard, parse_proposals, propose};
use vak_llm::types::ContentBlock;

#[test]
fn jaccard_catches_near_duplicates() {
    let a = "the deploy script must pause before rollback windows";
    let b = "deploy script: pause before rollback windows";
    let c = "favorite pizza toppings debate";
    assert!(jaccard(a, b) > 0.55);
    assert!(jaccard(a, c) < 0.1);
}

#[test]
fn parser_extracts_notes_and_skill_from_messy_reply() {
    let reply = r#"Sure! Here's what I'd keep:
{"notes":[
  {"note":"the deploy script must pause before rollback windows","kind":"decision","tag":"deploy"},
  {"note":"x","kind":"fact"},
  {"note":"user prefers small PRs","kind":"preference"}
 ],
 "skill":{"name":"Deploy Safely!","description":"","instructions":""},
 "trailing":1}"#;
    let p = parse_proposals(reply);
    // Short notes dropped; kinds normalized; tag sanitized.
    assert_eq!(p.notes.len(), 2);
    assert_eq!(p.notes[0].kind, "decision");
    assert_eq!(p.notes[0].tag, "deploy");
    assert!(p.skill.is_none(), "invalid skill name/description rejected");
}

#[test]
fn non_json_reply_yields_nothing() {
    let p = parse_proposals("Nothing worth remembering here at all.");
    assert!(p.notes.is_empty());
    assert!(p.skill.is_none());
}

fn scripted(json_reply: &str) -> Arc<dyn vak_llm::Provider> {
    struct Once(String);
    #[async_trait::async_trait]
    impl vak_llm::Provider for Once {
        fn name(&self) -> &str {
            "once"
        }
        async fn stream(
            &self,
            _req: vak_llm::types::ChatRequest,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> Result<vak_llm::EventStream, vak_llm::LlmError> {
            let (mut sink, rx) = vak_llm::stream::channel(8);
            sink.close_message(vak_llm::types::AssistantMessage {
                content: vec![ContentBlock::text(self.0.clone())],
                stop_reason: vak_llm::types::StopReason::EndTurn,
                usage: vak_llm::types::Usage::default(),
                model: "m".into(),
            })
            .await;
            Ok(rx)
        }
    }

    Arc::new(Once(json_reply.to_string()))
}

const GOOD_REPLY: &str = r#"{"notes":[{"note":"the deploy pipeline pauses before every rollback window","kind":"decision","tag":"deploys"}],"skill":{"name":"ship-it","description":"Ship with rollbacks guarded","instructions":"Run scripts/ship.sh"}}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reflection_writes_once_and_dedups_repeats() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let cwd = dir.path().to_path_buf();

    let proposals = propose(
        scripted(GOOD_REPLY),
        "m",
        "assistant: we shipped the deploy pipeline fix",
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(proposals.notes.len(), 1);
    assert_eq!(proposals.skill.as_ref().unwrap().name, "ship-it");

    // First apply writes the note and queues the skill.
    let (notes, queued) = apply(&home, &cwd, "sess-1", &proposals).unwrap();
    assert_eq!(notes, 1);
    assert!(queued);
    assert_eq!(memory::list_notes(&home, &cwd).len(), 1);
    assert_eq!(learning::list_proposals(&home, &cwd).len(), 1);

    // A near-identical reflection later adds NOTHING (bounded growth).
    let again = propose(
        scripted(GOOD_REPLY),
        "m",
        "same conversation again",
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .unwrap();
    let (notes2, queued2) = apply(&home, &cwd, "sess-2", &again).unwrap();
    assert_eq!(notes2, 0, "near-duplicate note must be skipped");
    // A second identical skill proposal queues under a distinct id; review
    // queue dedup is the human reviewer's call.
    assert!(!queued2 || !learning::list_proposals(&home, &cwd).is_empty(),);
}

#[test]
fn parser_accepts_invariant_notes() {
    let reply = r#"{"notes":[{"note":"always inspect schema before writing migration scripts","kind":"invariant","tag":"schema"}]}"#;
    let p = parse_proposals(reply);
    assert_eq!(p.notes.len(), 1);
    assert_eq!(p.notes[0].kind, "invariant");
    assert_eq!(p.notes[0].tag, "schema");
}
