//! What a turn's receipts establish about its outcome: the status of the
//! primary deliverable, each requirement, and the aggregate verdict. The
//! model never sets any of these (invariant 33).

use vak_intent::{
    CompletionVerdict, EvidenceReceipt, EvidenceState, OutcomeSpec, OutcomeStatus,
    RequirementEvaluation, RequirementImportance, RequirementKind, RequirementStatus,
};

/// Classifies whether a human should inspect the evaluated result. This is a
/// review signal, never an authorization decision.
pub fn human_review_state(verdict: CompletionVerdict) -> &'static str {
    match verdict {
        CompletionVerdict::Complete => "not_required",
        CompletionVerdict::Failed | CompletionVerdict::Cancelled => "required_for_recovery",
        CompletionVerdict::Partial | CompletionVerdict::Unknown => "recommended",
    }
}

pub fn evidence_state_from_age(
    now: chrono::DateTime<chrono::Utc>,
    recorded_at: chrono::DateTime<chrono::Utc>,
    max_age: chrono::Duration,
) -> EvidenceState {
    // Small clock skew is tolerated, but a receipt far in the future must not
    // become an automatically fresh proof.
    const MAX_CLOCK_SKEW_SECS: i64 = 300;
    if recorded_at > now + chrono::Duration::seconds(MAX_CLOCK_SKEW_SECS) {
        EvidenceState::Stale
    } else if now - recorded_at <= max_age {
        EvidenceState::Fresh
    } else {
        EvidenceState::Stale
    }
}

/// Evaluate freshness from a structured receipt. Source publication or event
/// time is preferred when present; observation time remains the legacy
/// fallback. This keeps retrieval of an old source from masquerading as a
/// current fact.
pub fn evidence_state_from_receipt(
    now: chrono::DateTime<chrono::Utc>,
    receipt: &EvidenceReceipt,
    max_age: chrono::Duration,
) -> EvidenceState {
    evidence_state_from_age(now, receipt.freshness_timestamp(), max_age)
}

pub fn evaluate_completion(
    status: OutcomeStatus,
    evaluations: &[RequirementEvaluation],
    spec: &OutcomeSpec,
) -> CompletionVerdict {
    match status {
        OutcomeStatus::Failed => CompletionVerdict::Failed,
        OutcomeStatus::Cancelled => CompletionVerdict::Cancelled,
        OutcomeStatus::Unknown => CompletionVerdict::Unknown,
        OutcomeStatus::Produced => {
            let must = spec
                .requirements
                .iter()
                .filter(|requirement| requirement.importance == RequirementImportance::Must);
            let mut has_unknown = false;
            let mut has_unmet = false;
            for requirement in must {
                match evaluations
                    .iter()
                    .find(|evaluation| evaluation.requirement_id == requirement.id)
                    .map(|evaluation| evaluation.status)
                {
                    Some(RequirementStatus::Met) => {}
                    Some(RequirementStatus::Unmet) | None => has_unmet = true,
                    Some(RequirementStatus::Unknown) => has_unknown = true,
                }
            }
            if has_unmet {
                CompletionVerdict::Partial
            } else if has_unknown {
                CompletionVerdict::Unknown
            } else {
                CompletionVerdict::Complete
            }
        }
    }
}

pub fn evaluate_response(response: Option<&str>, failed: bool, cancelled: bool) -> OutcomeStatus {
    evaluate_response_with_failures(response, failed, cancelled, false)
}

/// What the turn's receipts show of the deliverables the model's own plan
/// lists by form.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlannedReceipts {
    /// The plan lists a file.
    pub planned_file: bool,
    /// A call delivered a file or changed the workspace.
    pub delivered: bool,
    /// Cards the plan lists, and cards recorded this turn.
    pub planned_cards: usize,
    pub cards_shown: usize,
}

impl PlannedReceipts {
    /// Whether every planned file and card has a receipt. A sentence that
    /// says a file was written or a card was shown is not one.
    pub fn observed(&self) -> bool {
        (!self.planned_file || self.delivered) && self.cards_shown >= self.planned_cards
    }
}

/// As `evaluate_response`, but downgraded to `Unknown` when a tool call
/// failed and no later call of the same tool succeeded in the turn. The
/// caller decides what counts as failed; the kind of error does not matter,
/// because an answer written after an unrecovered failure, even one that
/// reports it honestly, is not established evidence and must not be signed
/// `Produced`.
pub fn evaluate_response_with_failures(
    response: Option<&str>,
    failed: bool,
    cancelled: bool,
    unresolved_failure: bool,
) -> OutcomeStatus {
    if failed {
        return OutcomeStatus::Failed;
    }
    if cancelled {
        return OutcomeStatus::Cancelled;
    }
    if unresolved_failure {
        return OutcomeStatus::Unknown;
    }
    match response.map(str::trim) {
        Some(text) if !text.is_empty() => OutcomeStatus::Produced,
        _ => OutcomeStatus::Unknown,
    }
}

/// Structural oracle for tabular data (markdown tables or vak-table/vak-dataframe blocks).
#[allow(clippy::collapsible_if)]
pub fn verify_tabular_data(text: &str) -> Option<Result<String, String>> {
    // Check vak-table or vak-dataframe
    if let Some(start) = text
        .find("```vak-table")
        .or_else(|| text.find("```vak-dataframe"))
    {
        let after = &text[start..];
        if let Some(nl) = after.find('\n') {
            let json_part = &after[nl + 1..];
            if let Some(end) = json_part.find("```") {
                let json_str = json_part[..end].trim();
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(cols) = v.get("columns").and_then(|c| c.as_array()) {
                        let col_count = cols.len();
                        if let Some(rows) = v.get("rows").and_then(|r| r.as_array()) {
                            for (idx, row) in rows.iter().enumerate() {
                                if let Some(cells) = row.as_array() {
                                    if cells.len() != col_count {
                                        return Some(Err(format!(
                                            "tabular row {idx} has {} cells, expected {col_count}",
                                            cells.len()
                                        )));
                                    }
                                }
                            }
                            return Some(Ok(format!(
                                "structured table verified: {col_count} columns, {} rows",
                                rows.len()
                            )));
                        }
                    }
                }
            }
        }
    }

    // Check markdown tables
    let mut table_lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() > 1 {
            table_lines.push(trimmed);
        } else if !table_lines.is_empty() {
            if table_lines.len() >= 2 {
                break;
            } else {
                table_lines.clear();
            }
        }
    }

    if table_lines.len() >= 2 {
        fn parse_markdown_row(l: &str) -> Vec<&str> {
            l.trim_matches('|').split('|').map(|c| c.trim()).collect()
        }
        let header = parse_markdown_row(table_lines[0]);
        let sep = parse_markdown_row(table_lines[1]);
        let is_sep = sep
            .iter()
            .all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':'));
        if is_sep && header.len() == sep.len() && !header.is_empty() {
            let expected_cols = header.len();
            for (i, row_str) in table_lines.iter().skip(2).enumerate() {
                let cells = parse_markdown_row(row_str);
                if cells.len() != expected_cols {
                    return Some(Err(format!(
                        "markdown table row {} has {} columns, expected {expected_cols}",
                        i + 1,
                        cells.len()
                    )));
                }
            }
            return Some(Ok(format!(
                "markdown table verified: {expected_cols} columns, {} rows",
                table_lines.len() - 2
            )));
        }
    }

    None
}

/// Structural oracle for decision/comparison matrices.
#[allow(clippy::collapsible_if)]
pub fn verify_decision_matrix(text: &str) -> Option<Result<String, String>> {
    if let Some(start) = text
        .find("```vak-decision")
        .or_else(|| text.find("```vak-comparison"))
    {
        let after = &text[start..];
        if let Some(nl) = after.find('\n') {
            let json_part = &after[nl + 1..];
            if let Some(end) = json_part.find("```") {
                let json_str = json_part[..end].trim();
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(options) = v.get("options").and_then(|o| o.as_array()) {
                        if options.len() < 2 {
                            return Some(Err(
                                "decision matrix requires at least 2 options to compare".into(),
                            ));
                        }
                        for (idx, opt) in options.iter().enumerate() {
                            if opt.get("label").or_else(|| opt.get("name")).is_none() {
                                return Some(Err(format!(
                                    "option {idx} is missing a label or name"
                                )));
                            }
                        }
                        return Some(Ok(format!(
                            "decision matrix verified: {} options compared",
                            options.len()
                        )));
                    }
                }
            }
        }
    }

    let lower = text.to_ascii_lowercase();
    if lower.contains("decision matrix")
        || lower.contains("comparison matrix")
        || lower.contains("tradeoff analysis")
    {
        if let Some(tab_res) = verify_tabular_data(text) {
            return match tab_res {
                Ok(msg) => Some(Ok(format!(
                    "decision matrix verified via tabular layout ({msg})"
                ))),
                Err(err) => Some(Err(format!(
                    "decision matrix tabular structure malformed: {err}"
                ))),
            };
        }
    }

    None
}

/// Structural oracle for claim-to-citation integrity.
#[allow(clippy::collapsible_if)]
pub fn verify_claim_citations(text: &str) -> Option<Result<String, String>> {
    let mut refs = std::collections::HashSet::new();
    let mut defs = std::collections::HashSet::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("[^") {
            if let Some(colon_pos) = trimmed.find("]:") {
                let tag = &trimmed[..colon_pos + 1];
                defs.insert(tag.to_string());
            }
        }
        let mut rest = line;
        while let Some(pos) = rest.find("[^") {
            let after = &rest[pos..];
            if let Some(end_pos) = after.find(']') {
                let tag = &after[..end_pos + 1];
                if !tag.ends_with("]:") {
                    refs.insert(tag.to_string());
                }
                rest = &after[end_pos + 1..];
            } else {
                break;
            }
        }
    }

    if refs.is_empty() {
        return None;
    }

    let mut unlinked: Vec<_> = refs
        .iter()
        .filter(|r| !defs.contains(*r))
        .map(|s| s.as_str())
        .collect();
    unlinked.sort();

    if unlinked.is_empty() {
        Some(Ok(format!(
            "claim citations verified: {} citations linked",
            refs.len()
        )))
    } else {
        Some(Err(format!(
            "unlinked footnote citations: {}",
            unlinked.join(", ")
        )))
    }
}

/// What the runtime observed about one turn, each a typed fact and none read
/// out of the reply's words (invariant 33: the runtime evaluates, the model
/// never does).
#[derive(Debug, Clone, Copy)]
pub struct TurnFacts<'a> {
    /// The reply, which the integrity checks parse for tables and citations
    /// it carries, and which counts as content when present.
    pub response: Option<&'a str>,
    /// From the turn's successful tool receipts and their age.
    pub evidence_state: EvidenceState,
    /// A typed evidence card (a research synthesis or an evidence card) was
    /// recorded in the ledger this turn.
    pub evidence_card: bool,
    /// The turn's named deliverable was observed in a tool result. True when
    /// the request names none.
    pub deliverable_observed: bool,
}

impl<'a> TurnFacts<'a> {
    /// A turn that observed nothing beyond its reply.
    pub fn reply(response: Option<&'a str>) -> Self {
        Self {
            response,
            evidence_state: EvidenceState::None,
            evidence_card: false,
            deliverable_observed: true,
        }
    }
}

/// Evaluate the requirements from what the runtime observed, as typed
/// facts. Nothing here reads the reply for words: whether the reply refuses,
/// or cites, is not something its text can establish.
///
/// An `Evidence` requirement is never `Met` here, by design: this function
/// can see that a typed evidence card or a retrieval receipt exists and
/// whether the receipt is fresh, but not whether the source *supports the
/// claim*. That is a
/// judgement, and the runtime does not make it structurally — so a turn
/// held to `cited` or stronger evidence closes at best `Unknown` from this
/// evaluator, with review recommended, until a linked criterion (a
/// `Shell`/`FileContains` check, an external receipt, a human attestation)
/// establishes it through the commitment ledger.
pub fn evaluate_requirements(
    spec: &OutcomeSpec,
    facts: &TurnFacts<'_>,
) -> Vec<RequirementEvaluation> {
    let response = facts.response;
    let evidence_state = facts.evidence_state;
    let has_response = response.is_some_and(|text| !text.trim().is_empty());
    spec.requirements
        .iter()
        .map(|requirement| {
            let (status, reason) = match requirement.kind {
                RequirementKind::Deliverable if !facts.deliverable_observed => (
                    RequirementStatus::Unknown,
                    "the named deliverable was not observed in any tool result".into(),
                ),
                RequirementKind::Deliverable if has_response => (
                    RequirementStatus::Met,
                    "response content exists".into(),
                ),
                RequirementKind::Deliverable => (
                    RequirementStatus::Unmet,
                    "no response content was produced".into(),
                ),
                RequirementKind::Evidence if evidence_state == EvidenceState::Fresh => (
                    RequirementStatus::Unknown,
                    "successful retrieval receipt exists; support and freshness still require evaluation".into(),
                ),
                RequirementKind::Evidence if evidence_state == EvidenceState::Stale => (
                    RequirementStatus::Unknown,
                    "evidence receipt exists but is stale for this request".into(),
                ),
                RequirementKind::Evidence if !facts.evidence_card => (
                    RequirementStatus::Unmet,
                    "no retrieval receipt or evidence card was recorded for this turn".into(),
                ),
                RequirementKind::Evidence => (
                    RequirementStatus::Unknown,
                    "an evidence card exists, but support and freshness were not established"
                        .into(),
                ),
                RequirementKind::Constraint => (
                    RequirementStatus::Unknown,
                    "constraint applicability requires a linked result".into(),
                ),
                RequirementKind::Integrity if has_response => {
                    let text = response.unwrap_or_default();
                    let mut checks = Vec::new();
                    if let Some(tab) = verify_tabular_data(text) {
                        checks.push(tab);
                    }
                    if let Some(dec) = verify_decision_matrix(text) {
                        checks.push(dec);
                    }
                    if let Some(cit) = verify_claim_citations(text) {
                        checks.push(cit);
                    }
                    if checks.is_empty() {
                        (RequirementStatus::Met, "no structured domain violations found".into())
                    } else if checks.iter().all(|c| c.is_ok()) {
                        (RequirementStatus::Met, "all structured domain integrity checks passed".into())
                    } else {
                        let violations: Vec<_> = checks.into_iter().filter_map(|c| c.err()).collect();
                        (RequirementStatus::Unmet, format!("domain integrity check failed: {}", violations.join("; ")))
                    }
                }
                RequirementKind::Integrity => (
                    RequirementStatus::Unmet,
                    "no response content was produced to verify domain integrity".into(),
                ),
            };
            RequirementEvaluation {
                requirement_id: requirement.id.clone(),
                status,
                reason,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use vak_intent::{Evidence, Reading};

    #[test]
    fn a_planned_card_or_file_needs_its_receipt() {
        let mut receipts = PlannedReceipts::default();
        assert!(receipts.observed());
        receipts.planned_cards = 2;
        receipts.cards_shown = 1;
        assert!(!receipts.observed(), "one planned card was never shown");
        receipts.cards_shown = 2;
        assert!(receipts.observed());
        receipts.planned_file = true;
        assert!(!receipts.observed());
        receipts.delivered = true;
        assert!(receipts.observed());
    }

    #[test]
    fn produced_output_is_not_called_complete() {
        assert_eq!(
            evaluate_response(Some("answer"), false, false),
            OutcomeStatus::Produced
        );
    }

    #[test]
    fn a_deliverable_is_met_by_observation_never_by_wording() {
        let mut spec = OutcomeSpec::from_reading("create a report", &Reading::general(), 1);
        assert!(
            spec.merge_declared_requirement(
                "report",
                "deliverable",
                "create report.md",
                "must",
                None
            )
            .is_ok()
        );
        let unobserved = TurnFacts {
            deliverable_observed: false,
            ..TurnFacts::reply(Some("Done: report.md is saved."))
        };
        let evaluations = evaluate_requirements(&spec, &unobserved);
        assert_eq!(evaluations[1].status, RequirementStatus::Unknown);
        assert_eq!(
            evaluate_completion(OutcomeStatus::Produced, &evaluations, &spec),
            CompletionVerdict::Unknown
        );
        let observed = evaluate_requirements(&spec, &TurnFacts::reply(Some("I cannot do that.")));
        assert_eq!(
            observed[1].status,
            RequirementStatus::Met,
            "the reply's words are not read for a refusal"
        );
    }

    #[test]
    fn cited_output_remains_unknown_until_source_support_is_checked() {
        let reading = Reading {
            evidence: Evidence::Cited,
            ..Reading::general()
        };
        let spec = OutcomeSpec::from_reading("research", &reading, 1);
        let facts = TurnFacts {
            evidence_card: true,
            ..TurnFacts::reply(Some("claim"))
        };
        let evaluations = evaluate_requirements(&spec, &facts);
        assert_eq!(evaluations[0].status, RequirementStatus::Met);
        assert_eq!(evaluations[1].status, RequirementStatus::Unknown);
    }

    #[test]
    fn retrieval_receipt_is_not_mistaken_for_supported_evidence() {
        let reading = Reading {
            evidence: Evidence::Cited,
            ..Reading::general()
        };
        let spec = OutcomeSpec::from_reading("research", &reading, 1);
        let facts = TurnFacts {
            evidence_state: EvidenceState::Fresh,
            ..TurnFacts::reply(Some("claim"))
        };
        let evaluations = evaluate_requirements(&spec, &facts);
        assert_eq!(evaluations[1].status, RequirementStatus::Unknown);
        assert!(evaluations[1].reason.contains("receipt"));
    }

    #[test]
    fn missing_and_unlinked_evidence_are_distinct_failures() {
        let reading = Reading {
            evidence: Evidence::Cited,
            ..Reading::general()
        };
        let spec = OutcomeSpec::from_reading("research", &reading, 1);
        let missing = evaluate_requirements(&spec, &TurnFacts::reply(Some("plain answer")));
        assert_eq!(missing[1].status, RequirementStatus::Unmet);
        let by_url = evaluate_requirements(
            &spec,
            &TurnFacts::reply(Some("answer https://example.com and [^1]")),
        );
        assert_eq!(
            by_url[1].status,
            RequirementStatus::Unmet,
            "a URL the reply states is not a recorded source"
        );
        let unlinked = evaluate_requirements(
            &spec,
            &TurnFacts {
                evidence_card: true,
                ..TurnFacts::reply(Some("answer"))
            },
        );
        assert_eq!(unlinked[1].status, RequirementStatus::Unknown);
        assert!(unlinked[1].reason.contains("support and freshness"));
    }

    #[test]
    fn stale_evidence_is_unknown_and_explains_why() {
        let reading = Reading {
            evidence: Evidence::Cited,
            ..Reading::general()
        };
        let spec = OutcomeSpec::from_reading("current events", &reading, 1);
        let evaluations = evaluate_requirements(
            &spec,
            &TurnFacts {
                evidence_state: EvidenceState::Stale,
                ..TurnFacts::reply(Some("answer"))
            },
        );
        assert_eq!(evaluations[1].status, RequirementStatus::Unknown);
        assert!(evaluations[1].reason.contains("stale"));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn evidence_receipt_keeps_observation_and_source_times_distinct() {
        let observed = chrono::DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z")
            .expect("timestamp")
            .with_timezone(&chrono::Utc);
        let published = chrono::DateTime::parse_from_rfc3339("2025-12-31T00:00:00Z")
            .expect("timestamp")
            .with_timezone(&chrono::Utc);
        let receipt = EvidenceReceipt {
            id: "r1".into(),
            kind: "article".into(),
            producer: "web".into(),
            observed_at: observed,
            source_published_at: Some(published),
            effective_at: None,
            state: EvidenceState::Fresh,
        };
        assert_eq!(receipt.freshness_timestamp(), published);
        assert_eq!(receipt.observed_at, observed);
        assert_eq!(
            evidence_state_from_receipt(observed, &receipt, chrono::Duration::days(1)),
            EvidenceState::Stale
        );
    }

    #[test]
    fn evidence_freshness_is_deterministic_and_domain_configurable() {
        let now = chrono::Utc::now();
        assert_eq!(
            evidence_state_from_age(
                now,
                now - chrono::Duration::hours(1),
                chrono::Duration::hours(2)
            ),
            EvidenceState::Fresh
        );
        assert_eq!(
            evidence_state_from_age(
                now,
                now - chrono::Duration::hours(3),
                chrono::Duration::hours(2)
            ),
            EvidenceState::Stale
        );
        assert_eq!(
            evidence_state_from_age(
                now,
                now + chrono::Duration::minutes(1),
                chrono::Duration::hours(2)
            ),
            EvidenceState::Fresh
        );
    }
    #[test]
    fn completion_verdict_never_confuses_output_with_satisfaction() {
        let reading = Reading {
            evidence: Evidence::Cited,
            ..Reading::general()
        };
        let spec = OutcomeSpec::from_reading("research", &reading, 1);
        let facts = TurnFacts {
            evidence_card: true,
            ..TurnFacts::reply(Some("answer"))
        };
        let evaluations = evaluate_requirements(&spec, &facts);
        assert_eq!(
            evaluate_completion(OutcomeStatus::Produced, &evaluations, &spec),
            CompletionVerdict::Unknown
        );
    }

    #[test]
    fn completion_verdict_handles_partial_failed_and_cancelled_work() {
        let reading = Reading {
            evidence: Evidence::Cited,
            ..Reading::general()
        };
        let spec = OutcomeSpec::from_reading("research", &reading, 1);
        let partial = vec![
            RequirementEvaluation {
                requirement_id: "deliverable-1".into(),
                status: RequirementStatus::Met,
                reason: "content exists".into(),
            },
            RequirementEvaluation {
                requirement_id: "evidence-1".into(),
                status: RequirementStatus::Unmet,
                reason: "no linked evidence".into(),
            },
        ];
        assert_eq!(
            evaluate_completion(OutcomeStatus::Produced, &partial, &spec),
            CompletionVerdict::Partial
        );
        assert_eq!(
            evaluate_completion(OutcomeStatus::Failed, &[], &spec),
            CompletionVerdict::Failed
        );
        assert_eq!(
            evaluate_completion(OutcomeStatus::Cancelled, &[], &spec),
            CompletionVerdict::Cancelled
        );
    }

    #[test]
    fn review_state_is_deterministic() {
        assert_eq!(
            human_review_state(CompletionVerdict::Complete),
            "not_required"
        );
        assert_eq!(
            human_review_state(CompletionVerdict::Unknown),
            "recommended"
        );
        assert_eq!(
            human_review_state(CompletionVerdict::Failed),
            "required_for_recovery"
        );
    }
    #[test]
    fn domain_verification_oracles_validate_tables_matrices_citations() {
        // Tabular markdown: well-formed
        let valid_table = "| Col A | Col B |\n| --- | --- |\n| Val 1 | Val 2 |\n| Val 3 | Val 4 |";
        assert!(verify_tabular_data(valid_table).unwrap().is_ok());

        // Tabular markdown: ragged (col count mismatch)
        let ragged_table = "| Col A | Col B |\n| --- | --- |\n| Val 1 |\n| Val 3 | Val 4 |";
        assert!(verify_tabular_data(ragged_table).unwrap().is_err());

        // Tabular vak block
        let valid_block = "```vak-table\n{\"columns\": [\"A\", \"B\"], \"rows\": [[\"1\", \"2\"], [\"3\", \"4\"]]}\n```";
        assert!(verify_tabular_data(valid_block).unwrap().is_ok());

        let invalid_block =
            "```vak-table\n{\"columns\": [\"A\", \"B\"], \"rows\": [[\"1\"], [\"3\", \"4\"]]}\n```";
        assert!(verify_tabular_data(invalid_block).unwrap().is_err());

        // Decision matrix vak block
        let valid_matrix = "```vak-decision\n{\"options\": [{\"label\": \"Option A\"}, {\"label\": \"Option B\"}]}\n```";
        assert!(verify_decision_matrix(valid_matrix).unwrap().is_ok());

        let single_option_matrix =
            "```vak-decision\n{\"options\": [{\"label\": \"Option A\"}]}\n```";
        assert!(
            verify_decision_matrix(single_option_matrix)
                .unwrap()
                .is_err()
        );

        // Claim citations
        let valid_citations =
            "According to study[^1] and report[^2].\n\n[^1]: Reference one\n[^2]: Reference two";
        assert!(verify_claim_citations(valid_citations).unwrap().is_ok());

        let unlinked_citations = "According to study[^1] and missing[^3].\n\n[^1]: Reference one";
        assert!(verify_claim_citations(unlinked_citations).unwrap().is_err());

        // RequirementKind::Integrity evaluation
        let mut spec = OutcomeSpec::from_reading("produce analysis", &Reading::general(), 1);
        spec.merge_declared_requirement(
            "integ-1",
            "integrity",
            "verify data integrity",
            "must",
            None,
        )
        .unwrap();

        let good_eval = evaluate_requirements(&spec, &TurnFacts::reply(Some(valid_table)));
        let integ_eval = good_eval
            .iter()
            .find(|e| e.requirement_id == "integ-1")
            .unwrap();
        assert_eq!(integ_eval.status, RequirementStatus::Met);

        let bad_eval = evaluate_requirements(&spec, &TurnFacts::reply(Some(ragged_table)));
        let bad_integ = bad_eval
            .iter()
            .find(|e| e.requirement_id == "integ-1")
            .unwrap();
        assert_eq!(bad_integ.status, RequirementStatus::Unmet);
    }
}
