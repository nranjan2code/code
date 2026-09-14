//! The outcome contract carried by a turn and, when needed, durable work.

use serde::{Deserialize, Serialize};

/// A request that arrives after execution has begun. It is classified before
/// it can affect the plan; free-form text is never treated as an authority
/// change by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionKind {
    Status,
    Steer,
    AddRequirement,
    RemoveRequirement,
    Reprioritize,
    Replan,
    Pause,
    Resume,
    Cancel,
    Approve,
    Reject,
}

impl InterventionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Steer => "steer",
            Self::AddRequirement => "add_requirement",
            Self::RemoveRequirement => "remove_requirement",
            Self::Reprioritize => "reprioritize",
            Self::Replan => "replan",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Cancel => "cancel",
            Self::Approve => "approve",
            Self::Reject => "reject",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionDecision {
    Accepted,
    Queued,
    RequiresHuman,
    Rejected,
}

impl InterventionDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Queued => "queued",
            Self::RequiresHuman => "requires_human",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterventionRequest {
    pub request_id: String,
    pub kind: InterventionKind,
    pub text: String,
    pub source: String,
    pub target_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterventionEvaluation {
    pub request: InterventionRequest,
    pub decision: InterventionDecision,
    pub reason: String,
    pub creates_revision: bool,
}

/// Evaluate the control-plane handling of an intervention. This deliberately
/// does not inspect or grant permissions; effectful changes still go through
/// the permission engine and approval gates.
pub fn evaluate_intervention(request: InterventionRequest) -> InterventionEvaluation {
    let human = request.source == "human" || request.source == "operator";
    let (decision, reason, creates_revision) = match request.kind {
        InterventionKind::Status => (
            InterventionDecision::Accepted,
            "status is observational",
            false,
        ),
        InterventionKind::Cancel => (
            InterventionDecision::Accepted,
            "cancellation is fail-safe",
            false,
        ),
        InterventionKind::Pause => (
            InterventionDecision::Accepted,
            "pause preserves partial work",
            false,
        ),
        InterventionKind::Resume => (
            InterventionDecision::Accepted,
            "resume continues at the next safe boundary",
            false,
        ),
        InterventionKind::Approve | InterventionKind::Reject if human => (
            InterventionDecision::Accepted,
            "human control-plane decision recorded",
            false,
        ),
        InterventionKind::Replan
        | InterventionKind::Reprioritize
        | InterventionKind::AddRequirement
        | InterventionKind::RemoveRequirement
            if human =>
        {
            (
                InterventionDecision::Queued,
                "scope change is queued for a new plan revision",
                true,
            )
        }
        InterventionKind::Approve | InterventionKind::Reject => (
            InterventionDecision::RequiresHuman,
            "only a human can resolve this control-plane decision",
            false,
        ),
        InterventionKind::Replan
        | InterventionKind::Reprioritize
        | InterventionKind::AddRequirement
        | InterventionKind::RemoveRequirement => (
            InterventionDecision::RequiresHuman,
            "scope changes proposed by an agent require human review",
            false,
        ),
        InterventionKind::Steer => (
            InterventionDecision::Queued,
            "steering queued at the next safe boundary",
            false,
        ),
    };
    InterventionEvaluation {
        request,
        decision,
        reason: reason.into(),
        creates_revision,
    }
}

/// Conservative classification for UI and audit purposes. Authorization and
/// plan mutation remain runtime responsibilities.
pub fn classify_intervention(text: &str) -> InterventionKind {
    let normalized = text.trim().to_ascii_lowercase();
    if normalized == "status" || normalized.starts_with("status ") {
        InterventionKind::Status
    } else if normalized == "cancel" || normalized.starts_with("stop ") {
        InterventionKind::Cancel
    } else if normalized == "pause" || normalized.starts_with("pause ") {
        InterventionKind::Pause
    } else if normalized == "resume" || normalized.starts_with("resume ") {
        InterventionKind::Resume
    } else if normalized.starts_with("replan") || normalized.starts_with("change plan") {
        InterventionKind::Replan
    } else if normalized.starts_with("prioritize") || normalized.starts_with("reprioritize") {
        InterventionKind::Reprioritize
    } else if normalized.starts_with("remove requirement")
        || normalized.starts_with("drop requirement")
    {
        InterventionKind::RemoveRequirement
    } else if normalized.starts_with("add requirement") || normalized.starts_with("also ") {
        InterventionKind::AddRequirement
    } else {
        InterventionKind::Steer
    }
}

use crate::{Evidence, Reading};

/// Whether a requirement came from the request or was inferred by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementOrigin {
    Explicit,
    Inferred,
}

/// How strongly a requirement affects completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementImportance {
    Must,
    Prefer,
}

/// The kind of result a requirement concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementKind {
    Deliverable,
    Evidence,
    Constraint,
    Integrity,
}

/// A checkable expectation attached to one outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeRequirement {
    pub id: String,
    pub kind: RequirementKind,
    pub description: String,
    pub origin: RequirementOrigin,
    pub importance: RequirementImportance,
    #[serde(default)]
    pub target: Option<String>,
}

/// The user's requested result as understood at turn admission.
///
/// This is an interpretation record, not an authority grant. It may guide
/// execution and presentation, but permissions and evidence strength remain
/// owned by their existing runtime boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeSpec {
    pub schema_version: u16,
    /// Monotonic contract revision within a session. Revision zero is the
    /// admission baseline; revisions are append-only and never rewritten.
    #[serde(default)]
    pub revision: u64,
    pub objective: String,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub requirements: Vec<OutcomeRequirement>,
    pub resolver_version: u32,
    #[serde(default)]
    pub evidence_max_age_secs: Option<i64>,
    /// Maximum model turns admitted for this outcome, when the engagement
    /// resolver derived a cap.
    #[serde(default)]
    pub max_turns: Option<usize>,
}

/// Runtime status of the primary deliverable. Produced output is not itself
/// proof that every requirement was satisfied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    Produced,
    Failed,
    Cancelled,
    Unknown,
}

/// Aggregate runtime verdict. This is derived from runtime evidence and
/// requirement evaluations; model prose cannot set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionVerdict {
    Complete,
    Partial,
    Unknown,
    Failed,
    Cancelled,
}

/// Classifies whether a human should inspect the evaluated result. This is a
/// review signal, never an authorization decision.
pub fn human_review_state(verdict: CompletionVerdict) -> &'static str {
    match verdict {
        CompletionVerdict::Complete => "not_required",
        CompletionVerdict::Failed | CompletionVerdict::Cancelled => "required_for_recovery",
        CompletionVerdict::Partial | CompletionVerdict::Unknown => "recommended",
    }
}

/// Runtime verdict for one requirement. `Unknown` is intentionally available
/// when a structural check cannot establish semantic correctness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementStatus {
    Met,
    Unmet,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceState {
    None,
    Fresh,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceReceipt {
    pub id: String,
    pub kind: String,
    pub producer: String,
    pub observed_at: chrono::DateTime<chrono::Utc>,
    /// When the underlying source was published or last materially updated.
    /// This is distinct from retrieval/observation time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_published_at: Option<chrono::DateTime<chrono::Utc>>,
    /// When the fact or event actually occurred, if different from retrieval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_at: Option<chrono::DateTime<chrono::Utc>>,
    pub state: EvidenceState,
}

impl EvidenceReceipt {
    /// Timestamp appropriate for freshness checks. Requirement evaluators can
    /// choose publication/effective time explicitly; observation remains the
    /// safe default for legacy receipts.
    pub fn freshness_timestamp(&self) -> chrono::DateTime<chrono::Utc> {
        self.source_published_at
            .or(self.effective_at)
            .unwrap_or(self.observed_at)
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementEvaluation {
    pub requirement_id: String,
    pub status: RequirementStatus,
    pub reason: String,
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

/// As `evaluate_response`, but downgraded to `Unknown` when the supporting
/// tool calls failed with a correctable fault the runtime could not recover
/// within the run repair budget (i.e. the model was nudged/instructed to
/// repair and did not). A non-empty fallback answer after such a failure is
/// not established evidence, so it must not be signed `Produced`.
pub fn evaluate_response_with_failures(
    response: Option<&str>,
    failed: bool,
    cancelled: bool,
    unresolved_correctable: bool,
) -> OutcomeStatus {
    if failed {
        return OutcomeStatus::Failed;
    }
    if cancelled {
        return OutcomeStatus::Cancelled;
    }
    if unresolved_correctable {
        return OutcomeStatus::Unknown;
    }
    match response.map(str::trim) {
        Some(text) if !text.is_empty() => OutcomeStatus::Produced,
        _ => OutcomeStatus::Unknown,
    }
}

/// Evaluate only facts the runtime can establish from the response itself.
/// Semantic support, freshness and claim relevance remain unknown without
/// linked evidence records.
pub fn evaluate_requirements(
    spec: &OutcomeSpec,
    response: Option<&str>,
) -> Vec<RequirementEvaluation> {
    evaluate_requirements_with_evidence(spec, response, false)
}

/// A successful retrieval receipt proves execution, not truth, relevance, or
/// freshness; those remain `Unknown` until linked evidence is checked.
pub fn evaluate_requirements_with_evidence(
    spec: &OutcomeSpec,
    response: Option<&str>,
    successful_evidence_receipt: bool,
) -> Vec<RequirementEvaluation> {
    evaluate_requirements_with_state(
        spec,
        response,
        if successful_evidence_receipt {
            EvidenceState::Fresh
        } else {
            EvidenceState::None
        },
    )
}

/// Structural oracle for tabular data (markdown tables or vak-table/vak-dataframe blocks).
pub fn verify_tabular_data(text: &str) -> Option<Result<String, String>> {
    // Check vak-table or vak-dataframe
    if let Some(start) = text.find("```vak-table").or_else(|| text.find("```vak-dataframe")) {
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
            l.trim_matches('|')
                .split('|')
                .map(|c| c.trim())
                .collect()
        }
        let header = parse_markdown_row(table_lines[0]);
        let sep = parse_markdown_row(table_lines[1]);
        let is_sep = sep.iter().all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':'));
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
pub fn verify_decision_matrix(text: &str) -> Option<Result<String, String>> {
    if let Some(start) = text.find("```vak-decision").or_else(|| text.find("```vak-comparison")) {
        let after = &text[start..];
        if let Some(nl) = after.find('\n') {
            let json_part = &after[nl + 1..];
            if let Some(end) = json_part.find("```") {
                let json_str = json_part[..end].trim();
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(options) = v.get("options").and_then(|o| o.as_array()) {
                        if options.len() < 2 {
                            return Some(Err("decision matrix requires at least 2 options to compare".into()));
                        }
                        for (idx, opt) in options.iter().enumerate() {
                            if opt.get("label").or_else(|| opt.get("name")).is_none() {
                                return Some(Err(format!("option {idx} is missing a label or name")));
                            }
                        }
                        return Some(Ok(format!("decision matrix verified: {} options compared", options.len())));
                    }
                }
            }
        }
    }

    let lower = text.to_ascii_lowercase();
    if lower.contains("decision matrix") || lower.contains("comparison matrix") || lower.contains("tradeoff analysis") {
        if let Some(tab_res) = verify_tabular_data(text) {
            return match tab_res {
                Ok(msg) => Some(Ok(format!("decision matrix verified via tabular layout ({msg})"))),
                Err(err) => Some(Err(format!("decision matrix tabular structure malformed: {err}"))),
            };
        }
    }

    None
}

/// Structural oracle for claim-to-citation integrity.
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

    let mut unlinked: Vec<_> = refs.iter().filter(|r| !defs.contains(*r)).map(|s| s.as_str()).collect();
    unlinked.sort();

    if unlinked.is_empty() {
        Some(Ok(format!("claim citations verified: {} citations linked", refs.len())))
    } else {
        Some(Err(format!("unlinked footnote citations: {}", unlinked.join(", "))))
    }
}

/// Evaluate requirements using a concrete evidence receipt and its
/// requirement freshness window. Retrieval success alone is insufficient.
pub fn evaluate_requirements_with_receipt(
    spec: &OutcomeSpec,
    response: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
    receipt: Option<&EvidenceReceipt>,
) -> Vec<RequirementEvaluation> {
    let state = receipt
        .map(|value| {
            let max_age = chrono::Duration::seconds(spec.evidence_max_age_secs.unwrap_or(86_400));
            evidence_state_from_receipt(now, value, max_age)
        })
        .unwrap_or(EvidenceState::None);
    evaluate_requirements_with_state(spec, response, state)
}

pub fn evaluate_requirements_with_state(
    spec: &OutcomeSpec,
    response: Option<&str>,
    evidence_state: EvidenceState,
) -> Vec<RequirementEvaluation> {
    let has_response = response.is_some_and(|text| !text.trim().is_empty());
    let is_refusal = response.is_some_and(|text| {
        let normalized = text.trim().to_ascii_lowercase();
        [
            "i cannot",
            "i can't",
            "i can’t",
            "unable to",
            "cannot do",
            "can't do",
            "can’t do",
        ]
        .iter()
        .any(|prefix| normalized.starts_with(prefix))
    });
    let has_structured_evidence = response.is_some_and(|text| {
        text.contains("\"semantic_type\":\"research.synthesis\"")
            || text.contains("\"semantic_type\": \"research.synthesis\"")
            || text.contains("\"semantic_type\":\"evidence\"")
            || text.contains("\"semantic_type\": \"evidence\"")
    });
    let has_reference = response.is_some_and(|text| {
        text.contains("http://")
            || text.contains("https://")
            || text.contains("[^")
    }) || has_structured_evidence;
    spec.requirements
        .iter()
        .map(|requirement| {
            let (status, reason) = match requirement.kind {
                RequirementKind::Deliverable if is_refusal => (
                    RequirementStatus::Unknown,
                    "response is a refusal; the requested deliverable was not established".into(),
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
                RequirementKind::Evidence if !has_reference => (
                    RequirementStatus::Unmet,
                    "no source reference was found in the response".into(),
                ),
                RequirementKind::Evidence => (
                    RequirementStatus::Unknown,
                    "a source reference exists, but support and freshness were not established"
                        .into(),
                ),
                RequirementKind::Constraint => (
                    RequirementStatus::Unknown,
                    "constraint applicability requires a linked result".into(),
                ),
                RequirementKind::Integrity if is_refusal => (
                    RequirementStatus::Unknown,
                    "response is a refusal; domain integrity check bypassed".into(),
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

impl OutcomeSpec {
    /// Build the conservative baseline contract for an ordinary turn.
    ///
    /// The request text is preserved as the objective; inferred requirements
    /// never grant tools or claim that evidence exists.
    pub fn from_reading(
        objective: impl Into<String>,
        reading: &Reading,
        resolver_version: u32,
    ) -> Self {
        let mut requirements = vec![OutcomeRequirement {
            id: "deliverable-1".into(),
            kind: RequirementKind::Deliverable,
            description: format!("produce an {} result", reading.act.as_str()),
            origin: RequirementOrigin::Inferred,
            importance: RequirementImportance::Must,
            target: Some("primary".into()),
        }];
        if reading.evidence != Evidence::None {
            requirements.push(OutcomeRequirement {
                id: "evidence-1".into(),
                kind: RequirementKind::Evidence,
                description: format!("meet the {} evidence standard", reading.evidence.as_str()),
                origin: RequirementOrigin::Inferred,
                importance: RequirementImportance::Must,
                target: Some("primary".into()),
            });
        }
        OutcomeSpec {
            schema_version: 1,
            revision: 0,
            objective: objective.into(),
            assumptions: Vec::new(),
            requirements,
            resolver_version,
            evidence_max_age_secs: match reading.evidence {
                Evidence::None => None,
                Evidence::Cited => Some(86_400),
                Evidence::Verified | Evidence::Audited => Some(3_600),
            },
            max_turns: None,
        }
    }

    /// Whether this outcome requires execution or file modifications.
    pub fn requires_execution(&self) -> bool {
        self.requirements.iter().any(|r| {
            r.kind == RequirementKind::Deliverable
                && (r.description.contains("modify")
                    || r.description.contains("author")
                    || r.description.contains("operate")
                    || r.description.contains("govern")
                    || r.description.contains("orchestrate"))
        })
    }

    /// Whether this outcome requires inspection, search, or enumeration.
    pub fn requires_inspection(&self) -> bool {
        self.requirements
            .iter()
            .any(|r| r.kind == RequirementKind::Deliverable && r.description.contains("locate"))
    }

    /// Whether this outcome requires real tool execution or evidence receipts.
    pub fn requires_tool(&self) -> bool {
        self.requires_execution()
            || self.requires_inspection()
            || self
                .requirements
                .iter()
                .any(|r| r.kind == RequirementKind::Evidence)
    }

    /// Extract the primary act description for logging and nudges.
    pub fn deliverable_act(&self) -> Option<&str> {
        self.requirements.iter().find_map(|r| {
            if r.kind == RequirementKind::Deliverable {
                let prefix = "produce an ";
                let suffix = " result";
                if let Some(act) = r
                    .description
                    .strip_prefix(prefix)
                    .and_then(|rest| rest.strip_suffix(suffix))
                {
                    return Some(act);
                }
                Some(r.description.as_str())
            } else {
                None
            }
        })
    }

    /// Merge an extension-provided requirement without allowing it to alter
    /// authority. Invalid declarations are rejected at the contract boundary.
    pub fn merge_declared_requirement(
        &mut self,
        id: impl Into<String>,
        kind: &str,
        description: impl Into<String>,
        importance: &str,
        target: Option<String>,
    ) -> Result<(), String> {
        let kind = match kind {
            "deliverable" => RequirementKind::Deliverable,
            "evidence" => RequirementKind::Evidence,
            "constraint" => RequirementKind::Constraint,
            "integrity" => RequirementKind::Integrity,
            other => return Err(format!("unsupported outcome requirement kind: {other}")),
        };
        let importance = match importance {
            "" | "prefer" => RequirementImportance::Prefer,
            "must" => RequirementImportance::Must,
            other => {
                return Err(format!(
                    "unsupported outcome requirement importance: {other}"
                ));
            }
        };
        let id = id.into();
        let description = description.into();
        if id.trim().is_empty() || description.trim().is_empty() {
            return Err("outcome requirement id and description are required".into());
        }
        if self
            .requirements
            .iter()
            .any(|requirement| requirement.id == id)
        {
            return Err(format!("duplicate outcome requirement id: {id}"));
        }
        self.requirements.push(OutcomeRequirement {
            id,
            kind,
            description,
            origin: RequirementOrigin::Inferred,
            importance,
            target,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Act, Reading};

    #[test]
    fn baseline_contract_preserves_objective_and_adds_only_inferred_requirements() {
        let reading = Reading {
            act: Act::Answer,
            evidence: Evidence::Cited,
            ..Reading::general()
        };
        let spec = OutcomeSpec::from_reading("current events in India", &reading, 1);
        assert_eq!(spec.objective, "current events in India");
        assert!(spec.requirements.iter().any(|requirement| {
            requirement.kind == RequirementKind::Evidence
                && requirement.origin == RequirementOrigin::Inferred
                && requirement.importance == RequirementImportance::Must
        }));
        assert!(
            spec.requirements
                .iter()
                .all(|requirement| { requirement.origin == RequirementOrigin::Inferred })
        );
    }

    #[test]
    fn produced_output_is_not_called_complete() {
        assert_eq!(
            evaluate_response(Some("answer"), false, false),
            OutcomeStatus::Produced
        );
    }

    #[test]
    fn refusal_cannot_satisfy_a_deliverable_requirement() {
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
        let evaluations = evaluate_requirements(&spec, Some("I cannot do that."));
        assert_eq!(evaluations[1].status, RequirementStatus::Unknown);
        assert_eq!(
            evaluate_completion(OutcomeStatus::Produced, &evaluations, &spec),
            CompletionVerdict::Unknown
        );
    }

    #[test]
    fn cited_output_remains_unknown_until_source_support_is_checked() {
        let reading = Reading {
            evidence: Evidence::Cited,
            ..Reading::general()
        };
        let spec = OutcomeSpec::from_reading("research", &reading, 1);
        let evaluations = evaluate_requirements(&spec, Some("claim https://example.com"));
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
        let evaluations =
            evaluate_requirements_with_evidence(&spec, Some("claim https://example.com"), true);
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
        let missing = evaluate_requirements(&spec, Some("plain answer"));
        assert_eq!(missing[1].status, RequirementStatus::Unmet);
        let unlinked = evaluate_requirements(&spec, Some("answer https://example.com"));
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
        let evaluations = evaluate_requirements_with_state(
            &spec,
            Some("answer https://example.com"),
            EvidenceState::Stale,
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
    fn extension_requirements_are_data_driven_and_narrowing_only() {
        let mut spec = OutcomeSpec::from_reading("make a plan", &Reading::general(), 1);
        let result = spec.merge_declared_requirement(
            "plan-structure",
            "constraint",
            "include assumptions and next steps",
            "must",
            Some("primary".into()),
        );
        assert!(result.is_ok());
        assert_eq!(
            spec.requirements.last().map(|item| item.origin),
            Some(RequirementOrigin::Inferred)
        );
        assert!(
            spec.merge_declared_requirement("", "constraint", "x", "must", None)
                .is_err()
        );
        assert!(
            spec.merge_declared_requirement("bad", "grant", "x", "must", None)
                .is_err()
        );
    }

    #[test]
    fn completion_verdict_never_confuses_output_with_satisfaction() {
        let reading = Reading {
            evidence: Evidence::Cited,
            ..Reading::general()
        };
        let spec = OutcomeSpec::from_reading("research", &reading, 1);
        let evaluations = evaluate_requirements(&spec, Some("answer https://example.com"));
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
    fn intervention_classification_is_conservative_and_auditable() {
        assert_eq!(
            classify_intervention("status please"),
            InterventionKind::Status
        );
        assert_eq!(
            classify_intervention("replan around the new constraint"),
            InterventionKind::Replan
        );
        assert_eq!(
            classify_intervention("also include a CSV"),
            InterventionKind::AddRequirement
        );
        assert_eq!(
            classify_intervention("remove requirement intervention-1"),
            InterventionKind::RemoveRequirement
        );
        assert_eq!(
            classify_intervention("use a shorter answer"),
            InterventionKind::Steer
        );
    }

    #[test]
    fn intervention_policy_requires_humans_for_agent_scope_changes() {
        let request = InterventionRequest {
            request_id: "i-1".into(),
            kind: InterventionKind::Replan,
            text: "replan".into(),
            source: "agent".into(),
            target_revision: Some(1),
        };
        let evaluation = evaluate_intervention(request);
        assert_eq!(evaluation.decision, InterventionDecision::RequiresHuman);
        assert!(!evaluation.creates_revision);
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

        let invalid_block = "```vak-table\n{\"columns\": [\"A\", \"B\"], \"rows\": [[\"1\"], [\"3\", \"4\"]]}\n```";
        assert!(verify_tabular_data(invalid_block).unwrap().is_err());

        // Decision matrix vak block
        let valid_matrix = "```vak-decision\n{\"options\": [{\"label\": \"Option A\"}, {\"label\": \"Option B\"}]}\n```";
        assert!(verify_decision_matrix(valid_matrix).unwrap().is_ok());

        let single_option_matrix = "```vak-decision\n{\"options\": [{\"label\": \"Option A\"}]}\n```";
        assert!(verify_decision_matrix(single_option_matrix).unwrap().is_err());

        // Claim citations
        let valid_citations = "According to study[^1] and report[^2].\n\n[^1]: Reference one\n[^2]: Reference two";
        assert!(verify_claim_citations(valid_citations).unwrap().is_ok());

        let unlinked_citations = "According to study[^1] and missing[^3].\n\n[^1]: Reference one";
        assert!(verify_claim_citations(unlinked_citations).unwrap().is_err());

        // RequirementKind::Integrity evaluation
        let mut spec = OutcomeSpec::from_reading("produce analysis", &Reading::general(), 1);
        spec.merge_declared_requirement("integ-1", "integrity", "verify data integrity", "must", None).unwrap();

        let good_eval = evaluate_requirements_with_state(&spec, Some(valid_table), EvidenceState::None);
        let integ_eval = good_eval.iter().find(|e| e.requirement_id == "integ-1").unwrap();
        assert_eq!(integ_eval.status, RequirementStatus::Met);

        let bad_eval = evaluate_requirements_with_state(&spec, Some(ragged_table), EvidenceState::None);
        let bad_integ = bad_eval.iter().find(|e| e.requirement_id == "integ-1").unwrap();
        assert_eq!(bad_integ.status, RequirementStatus::Unmet);
    }
}
