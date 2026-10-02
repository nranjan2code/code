#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde::Deserialize;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use vak_llm::{CapacityObservation, LlmError, RateLimitGate};

const FIXTURE: &str = include_str!("fixtures/provider_capacity_growth.json");

#[derive(Deserialize)]
struct Fixture {
    short_window: Window,
    daily_window: Window,
    events: Vec<Event>,
}

#[derive(Deserialize)]
struct Window {
    combined_tokens_limit: u64,
    reset_after_seconds: u64,
    #[serde(default)]
    remaining_before_refill: Option<u64>,
}

#[derive(Deserialize)]
struct Event {
    kind: String,
    duration_ms: Option<u64>,
    estimated_input_tokens: Option<u64>,
    reserved_output_tokens: Option<u64>,
    settled_input_tokens: Option<u64>,
    settled_output_tokens: Option<u64>,
    model_work_ms: Option<u64>,
    seconds: Option<u64>,
    expected: Option<String>,
}

fn demand(event: &Event) -> (u64, u64) {
    (
        event.estimated_input_tokens.expect("input estimate"),
        event.reserved_output_tokens.expect("output estimate"),
    )
}

#[tokio::test(start_paused = true)]
async fn content_free_growth_fixture_waits_once_then_requests_replan() {
    let fixture: Fixture = serde_json::from_str(FIXTURE).expect("fixture is valid JSON");
    let tool_wait_ms = fixture
        .events
        .iter()
        .filter(|event| event.kind == "tool_wait")
        .map(|event| event.duration_ms.expect("tool wait duration"))
        .sum::<u64>();
    let model_work_ms = fixture
        .events
        .iter()
        .filter(|event| event.kind == "model_attempt")
        .map(|event| event.model_work_ms.expect("model work duration"))
        .sum::<u64>();
    assert_eq!(tool_wait_ms, 100);
    assert_eq!(model_work_ms, 175);

    let account = unique_key("growth-account");
    let model = "synthetic-model";
    let model_gate = RateLimitGate::for_model(account.clone(), model);
    model_gate.observe(CapacityObservation {
        tokens_remaining: Some(fixture.short_window.combined_tokens_limit),
        tokens_limit: Some(fixture.short_window.combined_tokens_limit),
        reset_after_secs: Some(fixture.short_window.reset_after_seconds),
        daily_remaining: Some(fixture.daily_window.combined_tokens_limit),
        daily_limit: Some(fixture.daily_window.combined_tokens_limit),
        daily_reset_after_secs: Some(fixture.daily_window.reset_after_seconds),
        ..Default::default()
    });

    let attempts: Vec<_> = fixture
        .events
        .iter()
        .filter(|event| event.kind == "model_attempt")
        .collect();
    let (input, output) = demand(attempts[0]);
    let mut first = model_gate
        .reserve_demand(input, output, &CancellationToken::new())
        .await
        .expect("first request fits the observed window");
    first.settle(
        attempts[0].settled_input_tokens.expect("settled input"),
        attempts[0].settled_output_tokens.expect("settled output"),
    );

    let (input, output) = demand(attempts[1]);
    let queued_gate = model_gate.clone();
    let queued = tokio::spawn(async move {
        queued_gate
            .reserve_demand(input, output, &CancellationToken::new())
            .await
    });
    tokio::task::yield_now().await;
    assert!(
        !queued.is_finished(),
        "the second request waits for TPM refill"
    );
    tokio::time::advance(Duration::from_secs(
        fixture.short_window.reset_after_seconds,
    ))
    .await;
    let mut second = queued
        .await
        .expect("quota waiter task completes")
        .expect("second request fits after the short window refills");
    second.settle(
        attempts[1].settled_input_tokens.expect("settled input"),
        attempts[1].settled_output_tokens.expect("settled output"),
    );

    let growth = fixture
        .events
        .iter()
        .find(|event| event.kind == "context_growth")
        .expect("context growth event");
    let (input, output) = demand(growth);
    assert_eq!(growth.expected.as_deref(), Some("context_replan"));
    assert!(matches!(
        model_gate
            .reserve_demand(input, output, &CancellationToken::new())
            .await,
        Err(LlmError::Context(_))
    ));

    let daily_probe = fixture
        .events
        .iter()
        .find(|event| event.kind == "daily_window_probe")
        .expect("daily window probe event");
    let (input, output) = demand(daily_probe);
    let daily_gate = RateLimitGate::for_model(unique_key("daily-account"), model);
    daily_gate.observe(CapacityObservation {
        daily_remaining: fixture.daily_window.remaining_before_refill,
        daily_limit: Some(fixture.daily_window.combined_tokens_limit),
        daily_reset_after_secs: Some(fixture.daily_window.reset_after_seconds),
        ..Default::default()
    });
    let queued_daily_gate = daily_gate.clone();
    let daily_waiter = tokio::spawn(async move {
        queued_daily_gate
            .reserve_demand(input, output, &CancellationToken::new())
            .await
    });
    tokio::task::yield_now().await;
    assert!(
        !daily_waiter.is_finished(),
        "daily demand waits for its own observed reset"
    );
    tokio::time::advance(Duration::from_secs(
        fixture.daily_window.reset_after_seconds,
    ))
    .await;
    daily_waiter
        .await
        .expect("daily waiter task completes")
        .expect("daily demand fits after its quota resets");

    let retry_after = fixture
        .events
        .iter()
        .find(|event| event.kind == "provider_retry_after")
        .and_then(|event| event.seconds)
        .expect("provider retry delay");
    let account_gate = RateLimitGate::for_key(account);
    account_gate.observe_limit(Some(Duration::from_secs(retry_after)), Duration::ZERO);
    let waiting_gate = account_gate.clone();
    let wait = tokio::spawn(async move {
        waiting_gate
            .wait(&CancellationToken::new())
            .await
            .expect("Retry-After wait completes")
    });
    tokio::task::yield_now().await;
    assert!(!wait.is_finished(), "Retry-After is a provider wait");
    tokio::time::advance(Duration::from_secs(retry_after)).await;
    assert!(wait.await.expect("Retry-After task completes"));
}

fn unique_key(label: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!("fixture-{label}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}
