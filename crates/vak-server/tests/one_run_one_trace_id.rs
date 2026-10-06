//! M5 exit test `one_run_one_trace_id` (data-architecture plan M5b): one
//! run's span tree, `run › turn › step › (dispatch | tool_call ›
//! execution) › delivery`, shares one `trace_id`, including the
//! `execution` span the sandboxed tool worker writes in its own process and
//! hands back. It installs a process-wide subscriber, so it is a test
//! binary of its own.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};

struct Scripted {
    capacity_key: crate::support::CapacityKey,
    responses: Mutex<VecDeque<AssistantMessage>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
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

fn message(content: Vec<ContentBlock>, stop_reason: StopReason) -> AssistantMessage {
    AssistantMessage {
        content,
        stop_reason,
        usage: Usage {
            input_tokens: 7,
            output_tokens: 3,
            ..Default::default()
        },
        model: "test-model".into(),
        response_id: None,
    }
}

fn closes<'a>(lines: &'a [serde_json::Value], name: &str) -> Vec<&'a serde_json::Value> {
    lines
        .iter()
        .filter(|line| line["event"] == "span.close" && line["span"] == name)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_run_one_trace_id() {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    tracing::subscriber::set_global_default(vak_telemetry::capture(buffer.clone())).unwrap();

    let provider = Arc::new(Scripted {
        capacity_key: crate::support::CapacityKey::default(),
        responses: Mutex::new(VecDeque::from(vec![
            message(
                vec![ContentBlock::ToolUse {
                    id: "t1".into(),
                    name: "glob".into(),
                    input: serde_json::json!({"pattern": "*.md"}),
                }],
                StopReason::ToolUse,
            ),
            message(vec![ContentBlock::text("found them")], StopReason::EndTurn),
        ])),
    });
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        "permission_mode = \"full-access\"\n[memory]\nreflection = false\n[gateway]\nchat_allowlist_open = true\n",
    )
    .unwrap();
    std::fs::write(cwd.join("README.md"), "# readme\n").unwrap();
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(dir.path().join("home")));
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    core.set_tool_worker_exe(std::path::PathBuf::from(env!(
        "CARGO_BIN_EXE_vak-tool-worker"
    )));
    core.set_provider_instance(provider);
    let effects = core.effects();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = vak_server::gateway_router(core);
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let res = client
        .post(format!("{base}/gateway/inbound"))
        .json(&serde_json::json!({
            "surface": "probe",
            "chat": "traced",
            "text": "Which markdown files are here?",
            "wait": true,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["text"], "found them", "{body}");

    // The turn chain settles after it answers; wait for the run's close.
    let mut lines = Vec::new();
    for _ in 0..100 {
        lines = vak_telemetry::lines(&buffer);
        if !closes(&lines, "run").is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let runs = closes(&lines, "run");
    assert_eq!(runs.len(), 1, "one run: {runs:?}");
    let trace = runs[0]["trace_id"].as_str().unwrap().to_string();

    // What the run sends is an effect of it, and its delivery span names it.
    let mut key = vak_session::trace::TraceKey::root(
        vak_session::trace::local::tenant(),
        vak_session::trace::local::space(&cwd),
        vak_session::trace::local::agent("vak"),
        vak_session::trace::Cause::Heartbeat,
    );
    key.run = trace.parse().unwrap();
    let job = vak_delivery::DeliveryJob {
        job_id: "j".into(),
        target: "log:traced".into(),
        kind: vak_delivery::DeliveryKind::Assistant,
        content: vak_delivery::DeliveryContent::Answer(vak_delivery::AnswerDraft::from_markdown(
            "found them",
        )),
        profile: vak_delivery::DeliveryProfile::plain("log"),
        skill_registry: None,
        trace: None,
        actor: None,
    };
    let effect = effects
        .prepare(vak_session::effects::Prepare::new(
            vak_session::effects::EffectKind::delivery("log:traced"),
            "log:traced",
            Some(key),
            serde_json::to_vec(&job).unwrap(),
        ))
        .unwrap();
    let res = client
        .post(format!("{base}/effects/{}/resend", effect.id))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success(), "{}", res.status());
    let lines = vak_telemetry::lines(&buffer);

    for name in [
        "turn",
        "step",
        "dispatch",
        "tool_call",
        "execution",
        "delivery",
    ] {
        let found = closes(&lines, name);
        assert!(!found.is_empty(), "no {name} span closed");
        for line in found {
            assert_eq!(line["trace_id"], trace.as_str(), "{name}: {line}");
        }
    }
    // The worker continued the call's span from its own process.
    let call = closes(&lines, "tool_call")[0];
    let execution = closes(&lines, "execution")[0];
    assert_eq!(execution["service"], "worker");
    assert_eq!(execution["parent_span"], call["span_id"]);
    let path: Vec<&str> = execution["spans"]
        .as_array()
        .unwrap()
        .iter()
        .map(|span| span["name"].as_str().unwrap())
        .collect();
    assert_eq!(path, ["run", "turn", "step", "tool_call"]);
    // Every line under the run carries its trace id.
    for line in &lines {
        let in_run = line["spans"]
            .as_array()
            .is_some_and(|spans| spans.first().is_some_and(|span| span["name"] == "run"));
        if in_run {
            assert_eq!(line["trace_id"], trace.as_str(), "{line}");
        }
    }
    std::mem::forget(dir);
}
