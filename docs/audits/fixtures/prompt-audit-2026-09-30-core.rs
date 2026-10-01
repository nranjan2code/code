#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::sync::Arc;
use vak_llm::types::ContentBlock;

struct Once;
#[async_trait::async_trait]
impl vak_llm::Provider for Once {
    fn name(&self) -> &str { "once" }
    async fn stream(&self, _req: vak_llm::types::ChatRequest, _c: tokio_util::sync::CancellationToken)
        -> Result<vak_llm::EventStream, vak_llm::LlmError> {
        let (mut sink, rx) = vak_llm::stream::channel(8);
        sink.close_message(vak_llm::types::AssistantMessage {
            content: vec![ContentBlock::text("{}".to_string())],
            stop_reason: vak_llm::types::StopReason::EndTurn,
            usage: vak_llm::types::Usage::default(), model: "m".into(), response_id: None,
        }).await;
        Ok(rx)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn probe_reflection_multibyte() {
    // 11_999 ASCII bytes then a 3-byte Devanagari char straddling byte 12_000.
    let transcript = format!("user: {}नमस्ते", "a".repeat(11_993));
    let handle = tokio::spawn(async move {
        vak_core::reflection::propose(Arc::new(Once), "m", &transcript, tokio_util::sync::CancellationToken::new()).await
    });
    println!("PROBE reflection multibyte => {:?}", handle.await.map(|r| r.is_ok()).map_err(|e| e.is_panic()));
}

#[test]
fn probe_contract_shape_from_prompt() {
    // What the authoring prompt describes: kind is one of shell, ..., semantic.
    let prompt_shaped = serde_json::json!({
        "criterion_id": "c1", "statement": "the letter is drafted", "kind": "semantic", "required": true
    });
    let r: Result<vak_session::types::WorkCriterion, _> = serde_json::from_value(prompt_shaped);
    println!("PROBE criterion kind as string => {:?}", r.map(|_| "ok").map_err(|e| e.to_string()));
}

#[test]
fn probe_sizes() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let core = vak_core::Core::new(dir.path().to_path_buf()).unwrap();
    let sys = core.system_prompt();
    println!("PROBE system_prompt chars={} (~{} tok)", sys.chars().count(), sys.chars().count() / 4);
    let mut total = 0usize;
    let mut rows = Vec::new();
    for t in core.agent_tools() {
        let n = t.description().chars().count() + t.schema().to_string().chars().count();
        total += n;
        rows.push((n, t.name().to_string()));
    }
    rows.sort();
    rows.reverse();
    for (n, name) in rows.iter().take(12) { println!("PROBE tool {name} chars={n} (~{} tok)", n / 4); }
    println!("PROBE tools={} total chars={} (~{} tok)", rows.len(), total, total / 4);
    std::fs::write(std::env::var("PROBE_OUT").unwrap(), &sys).unwrap();
}
