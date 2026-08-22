#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The general-purpose (non-coding) suite must always pass: research,
//! data analysis, writing, conversion, inventory, and error-recovery
//! through the production loop with only the provider scripted.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn general_suite_fully_green() {
    let mut failures = Vec::new();
    for case in vak_eval::general_suite() {
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
        "general eval regressions:\n{}",
        failures.join("\n")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn general_suite_records_token_metrics() {
    let mut saw_usage = false;
    for case in vak_eval::general_suite() {
        let report = vak_eval::run_case(&case).await;
        assert!(report.passed, "case {} must pass", report.task_id);
        if report.tokens_in > 0 && report.tokens_out > 0 {
            saw_usage = true;
        }
    }
    assert!(saw_usage, "token accounting must be recorded per case");
}
