#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The built-in suite must always pass: these are regression gates for the
//! harness itself, not model benchmarks. CI fails if any case regresses.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn builtin_suite_fully_green() {
    let mut failures = Vec::new();
    for case in vak_eval::builtin_suite() {
        let report = vak_eval::run_case(&case).await;
        if !report.passed {
            failures.push(format!(
                "{}: {:?}",
                report.task_id,
                report.error.as_deref().unwrap_or("verify failed")
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "eval regressions:\n{}",
        failures.join("\n")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exhausted_script_is_reported_not_hung() {
    let case = vak_eval::EvalCase {
        id: "exhausted".into(),
        description: "script ends before the loop stops requesting".into(),
        files: vec![],
        prompt: "loop forever".into(),
        script: vec![],
        outcome: None,
        verify: "true".into(),
    };
    let report = vak_eval::run_case(&case).await;
    // A green postcondition cannot hide a failed agent turn.
    assert!(report.error.is_some(), "exhaustion must be visible");
    assert!(!report.passed, "loop failure must fail the case");
    assert_eq!(report.outcome_status, "failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generated_general_scenarios_run_without_a_model_provider() {
    for index in 0..10 {
        let case = vak_eval::generated_scenario(20260930, index);
        let report = vak_eval::run_case(&case).await;
        assert!(report.passed, "{}: {:?}", report.task_id, report.error);
        assert_eq!(report.outcome_status, "produced", "{}", report.task_id);
        assert_eq!(
            report.verification_evidence, "observed_success",
            "{}",
            report.task_id
        );
    }
}
