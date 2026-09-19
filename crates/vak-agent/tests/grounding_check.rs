#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Regression coverage for the verified real bug: a retrieval-shaped tool
//! call (e.g. `tavily_search`) succeeds and returns real results, but the
//! model's next turn ignores them and answers with ungrounded, uncited
//! prose. `Agent::run` must give the model exactly one bounded repair turn
//! in that case instead of letting the ungrounded answer stand.
//!
//! Which calls count as retrieval is decided by `AgentConfig::retrieval_check`,
//! supplied by `Core` from what each capability *declares it serves* — never
//! from a tool's name or its output. These tests supply a check that declares
//! the tool `search` as web-serving, and separately prove the name is
//! irrelevant in both directions.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::PermissionEngine;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::context::ToolContext;
use vak_tools::{Tool, ToolOutput};

struct Scripted {
    responses: Mutex<VecDeque<AssistantMessage>>,
}

#[async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let next = self.responses.lock().unwrap().pop_front();
        let (mut sink, rx) = stream::channel(64);
        match next {
            Some(m) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            None => sink.close_error(LlmError::Parse("exhausted".into())).await,
        }
        Ok(rx)
    }
}

/// Stands in for any retrieval-type MCP tool (Tavily/Exa/Firecrawl/a future
/// one nobody's written yet) — real, dated, URL-bearing results, exactly
/// the shape observed in the live bug's session transcript.
struct FakeSearchTool;

#[async_trait]
impl Tool for FakeSearchTool {
    fn name(&self) -> &str {
        "search"
    }
    fn description(&self) -> &str {
        "fake search tool for tests"
    }
    fn schema(&self) -> Value {
        serde_json::json!({"type": "object", "properties": {"query": {"type": "string"}}})
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok(
            "Title: Parliament debates UPI fee proposal\nURL: https://example-news.in/upi-fee-debate\n\n\
             Title: RBI holds rates steady\nURL: https://example-news.in/rbi-rates\n",
        )
    }
}

fn search_call(id: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: "search".into(),
            input: serde_json::json!({"query": "top news in india right now"}),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

fn text_msg(t: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(t)],
        stop_reason: StopReason::EndTurn,
        usage: Usage {
            input_tokens: 1,
            output_tokens: 1,
            ..Default::default()
        },
        model: "test-model".into(),
        response_id: None,
    }
}

fn fenced_research_answer() -> String {
    "Here is what I found:\n\n```vak\n{\"semantic_type\":\"research.synthesis\",\"payload\":{\"sources\":[{\"title\":\"UPI fee debate\",\"url\":\"https://example-news.in/upi-fee-debate\"}],\"takeaways\":[{\"text\":\"Parliament is debating a UPI fee.\",\"citation_indices\":[0]}]}}\n```".into()
}

async fn build_agent(
    dir: &tempfile::TempDir,
    session_id: &str,
    responses: Vec<AssistantMessage>,
) -> Agent {
    build_agent_with_check(
        dir,
        session_id,
        responses,
        Some(Arc::new(|name: &str, _: &Value| name == "search")),
    )
    .await
}

async fn build_agent_with_check(
    dir: &tempfile::TempDir,
    session_id: &str,
    responses: Vec<AssistantMessage>,
    retrieval_check: Option<vak_agent::RetrievalCheck>,
) -> Agent {
    let header = SessionHeader {
        agent: None,
        session_id: session_id.into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "scripted".into(),
            model: "test-model".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "sys".into(),
            permission_mode: "full-access".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    };
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), session_id),
        header,
    )
    .unwrap();

    Agent::new(
        Arc::new(Scripted {
            responses: Mutex::new(VecDeque::from(responses)),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.tools = vec![Arc::new(FakeSearchTool)];
            cfg.retrieval_check = retrieval_check;
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            cfg
        },
    )
}

// Raw ledger, not the model-visible projection: these tests are about the
// repair loop's mechanics (how many drafts were tried, what a nudge said),
// which a closed turn's full record deliberately no longer preserves
// (docs/design/68-context-engine.md §10) — a rejected draft is never
// projected, and every step collapses into one trace+narration message.
fn assistant_texts(agent: &Agent) -> Vec<String> {
    futures::executor::block_on(async {
        agent
            .session
            .lock()
            .await
            .message_chain()
            .iter()
            .filter(|(_, m)| m.role == vak_llm::types::Role::Assistant)
            .flat_map(|(_, m)| m.content.iter())
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    })
}

#[tokio::test]
async fn ungrounded_answer_after_search_gets_one_repair_turn() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "grounding-repair",
        vec![
            search_call("s1"),
            // First answer ignores the search result entirely — the exact
            // observed bug shape.
            text_msg("I have already provided a summary. Key themes: politics, business."),
            // After the [grounding-check] nudge, the model does the right thing.
            text_msg(&fenced_research_answer()),
        ],
    )
    .await;

    let outcome = agent
        .run(
            "what are top news in india right now",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    let texts = assistant_texts(&agent);
    assert!(
        texts.iter().any(|t| t.contains("\"semantic_type\"")),
        "final session state must contain a grounded card, got: {texts:?}"
    );

    // The repair nudge itself must be visible in the session (for
    // diagnostics/audit), and must name the tool that was ignored.
    let user_texts: Vec<String> = futures::executor::block_on(async {
        agent
            .session
            .lock()
            .await
            .message_chain()
            .iter()
            .filter(|(_, m)| m.role == vak_llm::types::Role::User)
            .flat_map(|(_, m)| m.content.iter())
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    });
    assert!(
        user_texts
            .iter()
            .any(|t| t.contains("[grounding-check]") && t.contains("search")),
        "expected a grounding-check nudge naming the ignored tool, got: {user_texts:?}"
    );
}

#[tokio::test]
async fn repair_is_bounded_to_one_attempt() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "grounding-bounded",
        vec![
            search_call("s1"),
            text_msg("still vague, still no fence, first ignore"),
            // Model ignores the nudge too — must NOT loop forever; the run
            // completes (with the still-bad answer) rather than repeating.
            text_msg("still vague, still no fence, second ignore"),
        ],
    )
    .await;

    let outcome = agent
        .run(
            "what are top news in india right now",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    let texts = assistant_texts(&agent);
    assert_eq!(
        texts.iter().filter(|t| t.contains("still vague")).count(),
        2,
        "exactly one repair retry — not zero, not a loop: {texts:?}"
    );
}

#[tokio::test]
async fn honest_no_data_admission_is_not_flagged() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "grounding-honest",
        vec![
            search_call("s1"),
            text_msg(
                "I couldn't find current information on that from the search results — \
                 the results didn't cover today's headlines.",
            ),
        ],
    )
    .await;

    let outcome = agent
        .run(
            "what are top news in india right now",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    let texts = assistant_texts(&agent);
    assert_eq!(
        texts.iter().filter(|t| t.contains("couldn't find")).count(),
        1,
        "an honest admission of no data must NOT trigger a repair retry: {texts:?}"
    );
}

#[tokio::test]
async fn grounded_first_answer_needs_no_repair() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "grounding-clean",
        vec![search_call("s1"), text_msg(&fenced_research_answer())],
    )
    .await;

    let outcome = agent
        .run(
            "what are top news in india right now",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    let texts = assistant_texts(&agent);
    assert_eq!(
        texts.iter().filter(|t| t.contains("semantic_type")).count(),
        1,
        "a correctly-grounded first answer must not be re-run: {texts:?}"
    );
}

#[tokio::test]
async fn session_search_of_the_users_own_notes_is_not_flagged() {
    // Regression: `session_search` (searching the user's OWN session
    // history/notes, not external data) must never trip the grounding
    // check just because its name contains "search". This exact false
    // positive broke a real test (crates/vak-core/tests/learning_loop.rs
    // `remember_propose_recall_promote_loop`) before the fix.
    struct SessionSearchTool;
    #[async_trait]
    impl Tool for SessionSearchTool {
        fn name(&self) -> &str {
            "session_search"
        }
        fn description(&self) -> &str {
            "search the user's own session/notes history"
        }
        fn schema(&self) -> Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
            ToolOutput::ok(
                "1 hit(s): deploy-rollbacks — the deploy script must pause before rollback windows",
            )
        }
    }

    let dir = tempdir().unwrap();
    let header = SessionHeader {
        agent: None,
        session_id: "grounding-session-search".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "scripted".into(),
            model: "test-model".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "sys".into(),
            permission_mode: "full-access".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    };
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), "grounding-session-search"),
        header,
    )
    .unwrap();

    let mut agent = Agent::new(
        Arc::new(Scripted {
            responses: Mutex::new(VecDeque::from(vec![
                AssistantMessage {
                    content: vec![ContentBlock::ToolUse {
                        id: "s1".into(),
                        name: "session_search".into(),
                        input: serde_json::json!({"query": "deploy rollback"}),
                    }],
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                    model: "test-model".into(),
                    response_id: None,
                },
                text_msg("Recalled from memory."),
            ])),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.tools = vec![Arc::new(SessionSearchTool)];
            cfg.retrieval_check = Some(Arc::new(|name, _| name == "search"));
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            cfg
        },
    );

    let outcome = agent
        .run(
            "what do we know about deploys?",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "session_search must not trigger a grounding repair (and stall on an exhausted script): got {outcome:?}"
    );

    let texts = assistant_texts(&agent);
    assert_eq!(
        texts
            .iter()
            .filter(|t| t.contains("Recalled from memory"))
            .count(),
        1,
        "no repair retry should have fired: {texts:?}"
    );
}

#[tokio::test]
async fn ungrounded_prose_unrelated_to_any_tool_call_is_not_flagged() {
    // No search/retrieval tool ran this turn at all — an ordinary prose
    // answer must never be forced through the grounding check.
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "grounding-no-tool",
        vec![text_msg("The capital of France is Paris.")],
    )
    .await;

    let outcome = agent
        .run(
            "what is the capital of france",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    let texts = assistant_texts(&agent);
    assert_eq!(
        texts
            .iter()
            .filter(|t| t.contains("capital of France"))
            .count(),
        1,
        "no tool call happened, so no grounding retry should fire: {texts:?}"
    );
}

/// The agent never decides what is retrieval. A tool literally named `search`
/// whose results are full of URLs — exactly what the old keyword-and-URL rule
/// flagged — is not grounded on when the capability layer does not declare it
/// as reaching outside information.
#[tokio::test]
async fn a_tool_is_retrieval_only_if_the_check_says_so_whatever_its_name_or_output() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent_with_check(
        &dir,
        "grounding-name-irrelevant",
        vec![
            search_call("s1"),
            text_msg("Here is a summary with no citations at all."),
        ],
        Some(Arc::new(|_: &str, _: &Value| false)),
    )
    .await;
    let outcome = agent
        .run(
            "look something up",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );
    let users: Vec<String> = futures::executor::block_on(async {
        agent
            .session
            .lock()
            .await
            .derive_messages()
            .iter()
            .filter(|m| m.role == vak_llm::types::Role::User)
            .flat_map(|m| m.content.iter())
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    });
    assert!(
        !users.iter().any(|t| t.contains("[grounding-check]")),
        "an undeclared tool must never trigger a grounding nudge: {users:?}"
    );
}

/// With no check configured at all the grounding check is inert rather than
/// falling back to a guess.
#[tokio::test]
async fn with_no_retrieval_check_the_grounding_check_is_inert() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent_with_check(
        &dir,
        "grounding-inert",
        vec![search_call("s1"), text_msg("Uncited summary.")],
        None,
    )
    .await;
    let outcome = agent
        .run(
            "look something up",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );
}
