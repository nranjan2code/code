#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The general-purpose (non-coding) suite must always pass: research,
//! data analysis, writing, conversion, inventory, and error-recovery
//! through the production loop with only the provider scripted.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn general_suite_fully_green() {
    let mut failures = Vec::new();
    for case in vak_eval::general_suite() {
        let report = vak_eval::run_case(&case).await;
        assert_eq!(
            report.completion_verdict, "complete",
            "{} must satisfy its runtime outcome",
            report.task_id
        );
        assert_eq!(report.human_review, "not_required");
        assert_eq!(report.verification_evidence, "observed_success");
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

#[test]
fn general_suite_is_not_coding_only() {
    let cases = vak_eval::general_suite();
    assert!(
        cases.len() >= 5,
        "general-purpose corpus needs multiple domains"
    );
    let ids = cases
        .iter()
        .map(|case| case.id.to_ascii_lowercase())
        .collect::<Vec<_>>();
    for domain in [
        "research",
        "csv",
        "writing",
        "conversion",
        "inventory",
        "schedule",
        "decision",
    ] {
        assert!(
            ids.iter().any(|id| id.contains(domain)),
            "missing {domain} case"
        );
    }
}

#[test]
fn held_out_corpus_has_context_and_deliverable_variation() {
    let cases = vak_eval::held_out_suite();
    assert!(cases.len() >= 2);
    assert!(cases.iter().any(|case| case.prompt.contains("English")));
    assert!(cases.iter().any(|case| case.prompt.contains("one-line")));
    assert!(cases.iter().all(|case| !case.verify.is_empty()));
    assert!(cases.iter().any(|case| {
        case.outcome
            .as_ref()
            .is_some_and(|outcome| outcome.requirements.len() >= 2)
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn held_out_corpus_runs_through_the_production_loop() {
    for case in vak_eval::held_out_suite() {
        let report = vak_eval::run_case(&case).await;
        assert!(report.passed, "{} must pass", report.task_id);
        assert_eq!(report.verification_evidence, "observed_success");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_held_out_evidence_requirement_downgrades_verdict() {
    let case = vak_eval::held_out_suite()
        .into_iter()
        .find(|case| case.id == "held-out-multilingual-note")
        .expect("multilingual held-out case");
    let report = vak_eval::run_case(&case).await;
    assert!(report.passed);
    assert_eq!(report.completion_verdict, "partial");
    assert_eq!(report.human_review, "recommended");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn comparison_proves_outcome_directed_evidence_changes_verdict() {
    let case = vak_eval::held_out_suite()
        .into_iter()
        .find(|case| case.id == "held-out-multilingual-note")
        .expect("multilingual held-out case");
    let comparison = vak_eval::compare_case(&case).await;
    assert!(comparison.baseline.passed);
    assert!(comparison.outcome_directed.passed);
    assert_eq!(comparison.baseline.completion_verdict, "complete");
    assert_eq!(comparison.outcome_directed.completion_verdict, "partial");
    assert_eq!(comparison.outcome_directed.human_review, "recommended");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn held_out_comparison_reports_corpus_level_deltas() {
    let comparison = vak_eval::compare_suite(&vak_eval::held_out_suite()).await;
    assert_eq!(comparison.baseline.total, 3);
    assert_eq!(comparison.outcome_directed.total, 3);
    assert_eq!(comparison.baseline.passed, 3);
    assert_eq!(comparison.outcome_directed.passed, 3);
    assert_eq!(comparison.outcome_directed.human_review_recommended, 1);
    assert!(comparison.token_delta >= 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn held_out_corpus_reports_aggregate_measurement() {
    let report = vak_eval::run_suite(&vak_eval::held_out_suite()).await;
    assert_eq!(report.total, 3);
    assert_eq!(report.passed, report.total);
    assert_eq!(report.pass_rate, 1.0);
    assert!(report.total_tokens_in > 0);
    assert!(report.total_tokens_out > 0);
    assert_eq!(report.cases.len(), report.total);
    assert_eq!(report.partial_or_unknown, 1);
    assert_eq!(report.human_review_recommended, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_external_verification_cannot_claim_completion() {
    let case = vak_eval::EvalCase {
        id: "negative-verification".into(),
        description: "a plausible response with a failed external check".into(),
        files: vec![],
        prompt: "produce the requested result".into(),
        script: vec![vak_eval::ScriptedTurn::Text("The result is ready.".into())],
        outcome: None,
        verify: "false".into(),
    };
    let report = vak_eval::run_case(&case).await;
    assert!(!report.passed);
    assert_eq!(report.outcome_status, "produced");
    assert_eq!(report.completion_verdict, "partial");
    assert_eq!(report.human_review, "recommended");
    assert_eq!(report.verification_evidence, "observed_failure");
}
