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
    let mut case = vak_eval::EvalCase {
        id: "exhausted".into(),
        description: "script ends before the loop stops requesting".into(),
        files: vec![],
        prompt: "loop forever".into(),
        script: vec![],
        verify: "true".into(),
    };
    let _ = &mut case;
    let report = vak_eval::run_case(&case).await;
    // The scripted provider errors; the loop surfaces Failed; verify still
    // passes (`true`), but the error is recorded on the report.
    assert!(report.error.is_some(), "exhaustion must be visible");
}
