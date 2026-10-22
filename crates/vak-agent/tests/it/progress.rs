#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::card_gates::{build_agent, run, text_msg, tool_call_msg};
use vak_agent::TurnOutcome;
use vak_llm::ContentBlock;

fn list(state: &str) -> serde_json::Value {
    serde_json::json!({"items":[{"label":"Read the source","state":state,"reason":if state == "dropped" { Some("The source is unavailable") } else { None }}]})
}
async fn reminders(agent: &vak_agent::Agent) -> usize {
    agent.session.lock().await.chain_to_root().iter().filter(|entry|
        matches!(&entry.payload, vak_session::EntryPayload::Message(record) if record.control_kind() == Some(vak_intent::control::ControlKind::ProgressReminder))
    ).count()
}

#[tokio::test]
async fn open_progress_gets_one_reminder_and_the_next_answer_stands_without_discarding_text() {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, _) = build_agent(
        &dir,
        "progress-open",
        vec![
            tool_call_msg("p", "progress", list("pending")),
            text_msg("First useful result."),
            text_msg("This is the available answer."),
        ],
    )
    .await;
    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    assert_eq!(reminders(&agent).await, 1);
    let session = agent.session.lock().await;
    let messages: Vec<_> = session
        .message_chain()
        .into_iter()
        .map(|(_, message)| message)
        .collect();
    for text in ["First useful result.", "This is the available answer."] {
        assert!(
            messages
                .iter()
                .any(|message| message.text_content().contains(text))
        );
    }
    assert!(session.message_chain().iter().flat_map(|(_,message)| &message.content).any(|block|
        matches!(block,ContentBlock::ToolResult { tool_use_id,content,is_error:false } if tool_use_id=="p" && serde_json::from_str::<vak_intent::progress::ProgressList>(content).unwrap().open_labels()==["Read the source"])
    ));
    assert!(
        session.presentations().is_empty(),
        "progress never forces a card"
    );
}

#[tokio::test]
async fn done_dropped_and_absent_progress_get_no_reminder() {
    for state in [Some("done"), Some("dropped"), None] {
        let dir = tempfile::tempdir().unwrap();
        let mut script = Vec::new();
        if let Some(state) = state {
            script.push(tool_call_msg("p", "progress", list(state)));
        }
        script.push(text_msg("Available answer."));
        let (mut agent, _) = build_agent(&dir, "progress-closed", script).await;
        assert!(matches!(
            run(&mut agent).await,
            TurnOutcome::Completed { .. }
        ));
        assert_eq!(reminders(&agent).await, 0);
    }
}

#[tokio::test]
async fn the_latest_whole_list_controls_the_reminder_and_resets_for_the_next_turn() {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, _) = build_agent(
        &dir,
        "progress-update",
        vec![
            tool_call_msg("p1", "progress", list("in_progress")),
            tool_call_msg("p2", "progress", list("done")),
            text_msg("Finished."),
            text_msg("Direct answer."),
        ],
    )
    .await;
    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    assert_eq!(reminders(&agent).await, 0);
}

#[tokio::test]
async fn cards_do_not_match_progress_labels_and_are_never_refused_for_them() {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, _) = build_agent(
        &dir,
        "progress-card",
        vec![
            tool_call_msg("p", "progress", list("done")),
            tool_call_msg(
                "c",
                "emit_chart_card",
                serde_json::json!({"title":"An unrelated authored title","sources":[]}),
            ),
            text_msg("The chart is ready."),
        ],
    )
    .await;
    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    assert_eq!(reminders(&agent).await, 0);
    let session = agent.session.lock().await;
    assert!(!session.presentations().is_empty());
    assert!(
        session
            .presentations()
            .iter()
            .all(|(_, record)| !record.derived_from.iter().any(|id| id == "p")),
        "progress is not a card source"
    );
    assert!(session.message_chain().iter().flat_map(|(_,message)| &message.content).any(|block| matches!(block,ContentBlock::ToolResult { tool_use_id,is_error:false,.. } if tool_use_id=="c")));
}

#[tokio::test]
async fn a_progress_label_is_not_retrieved_evidence_for_a_card() {
    let dir = tempfile::tempdir().unwrap();
    let url = "https://not-retrieved.example/report";
    let (mut agent,_)=build_agent(&dir,"progress-not-evidence",vec![
        tool_call_msg("p","progress",serde_json::json!({"items":[{"label":format!("Review {url}"),"state":"done","reason":null}]})),
        tool_call_msg("c","emit_chart_card",serde_json::json!({"sources":[{"url":url}]})),
        text_msg("That source has not been retrieved."),
    ]).await;
    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    let session = agent.session.lock().await;
    assert!(session.presentations().is_empty());
    assert!(session.message_chain().iter().flat_map(|(_,message)| &message.content).any(|block| matches!(block,ContentBlock::ToolResult {tool_use_id,is_error:true,..} if tool_use_id=="c")));
}
