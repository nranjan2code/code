use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

/// Authored text is content. Runtime controls are filtered by their typed
/// ledger metadata before projection, never by matching words in prose.
pub(crate) fn clean_scaffolding(text: &str) -> String {
    text.trim().to_string()
}

use vak_agent::AgentEvent;
use vak_delivery::{
    ArtifactRef, ArtifactStatus, DeliveryAction, OutputContent, OutputItem, OutputKind,
    OutputProvenance, OutputRole, OutputStatus, OutputStreamEvent, OutputTimeline,
    PresentationDocument, ResultOutcome, SkillRegistry, compile_markdown, structured_markdown,
    structured_outputs_from_text_with,
};

fn status_for_completion(completion: Option<&str>) -> OutputStatus {
    if completion.is_none() || completion.is_some_and(|value| value.trim() == "complete") {
        OutputStatus::Succeeded
    } else {
        OutputStatus::Partial
    }
}

/// Outcome fields stay separate from authored evaluation text. Delimiters in
/// a question or a reason must never change completion, ids or receipt links.
#[derive(Debug, Clone)]
struct TurnEvaluation {
    status: String,
    evaluation: String,
    receipts: Vec<String>,
    completion: Option<String>,
    requirement_ids: Vec<String>,
}

impl TurnEvaluation {
    fn from_activity(activity: &vak_session::ActivityRecord) -> Self {
        let data = &activity.data;
        let evaluation = data
            .get("evaluation")
            .cloned()
            .unwrap_or_else(|| "[]".into());
        let decoded = serde_json::from_str::<Vec<vak_intent::RequirementEvaluation>>(&evaluation);
        let requirement_ids = decoded
            .as_ref()
            .map(|rows| rows.iter().map(|row| row.requirement_id.clone()).collect())
            .unwrap_or_default();
        Self {
            status: data
                .get("status")
                .cloned()
                .unwrap_or_else(|| "unknown".into()),
            evaluation,
            receipts: data
                .get("evidence_receipts")
                .into_iter()
                .flat_map(|ids| ids.split(','))
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect(),
            completion: if decoded.is_err() {
                Some("unknown".into())
            } else {
                data.get("completion").cloned()
            },
            requirement_ids,
        }
    }
}

fn result_outcome(
    result_id: impl Into<String>,
    status: OutputStatus,
    admitted: Option<&vak_intent::OutcomeSpec>,
    evaluation: Option<&TurnEvaluation>,
    evidence_state: Option<&String>,
    human_review: Option<&String>,
) -> ResultOutcome {
    let completion = evaluation.and_then(|value| value.completion.clone());
    ResultOutcome {
        result_id: result_id.into(),
        status,
        completion,
        evidence_state: evidence_state.cloned(),
        requirement_ids: evaluation
            .map(|value| value.requirement_ids.clone())
            .filter(|ids| !ids.is_empty())
            .unwrap_or_else(|| {
                admitted
                    .map(|outcome| {
                        outcome
                            .requirements
                            .iter()
                            .map(|item| item.id.clone())
                            .collect()
                    })
                    .unwrap_or_default()
            }),
        evidence_receipt_ids: evaluation
            .map(|value| value.receipts.clone())
            .unwrap_or_default(),
        evidence: Vec::new(),
        human_review: human_review.cloned(),
    }
}
/// Outcome belongs to answer material, including answers made only of cards.
/// User prompts are never a result anchor. Every form reads the same verdict.
fn apply_turn_evaluations(
    timeline: &mut OutputTimeline,
    evaluations: &HashMap<String, TurnEvaluation>,
    specs: &HashMap<String, vak_intent::OutcomeSpec>,
    evidence: &HashMap<String, String>,
    review: &HashMap<String, String>,
) {
    let is_material = |item: &OutputItem| {
        item.role != OutputRole::User
            && (item.kind == OutputKind::Card
                || item.role == OutputRole::Assistant
                    && matches!(
                        item.content,
                        OutputContent::Document { .. } | OutputContent::Adaptive { .. }
                    ))
    };
    let mut directives = HashMap::new();
    let mut results = HashMap::new();
    for item in &timeline.items {
        if item.role == OutputRole::User {
            if let Some(id) = item
                .provenance
                .as_ref()
                .and_then(|provenance| provenance.entry_id.as_ref())
            {
                directives.insert(item.turn_id.clone(), id.clone());
            }
        } else if is_material(item) {
            if let Some(outcome) = &item.outcome {
                results.insert(item.turn_id.clone(), outcome.result_id.clone());
            } else {
                results
                    .entry(item.turn_id.clone())
                    .or_insert_with(|| item.id.clone());
            }
        }
    }
    for item in &mut timeline.items {
        if item.role == OutputRole::User {
            item.outcome = None;
            item.status = OutputStatus::Succeeded;
            if let OutputContent::Document { document } = &mut item.content {
                document
                    .metadata
                    .retain(|key, _| !key.starts_with("outcome_"));
            }
            continue;
        }
        if !is_material(item) {
            continue;
        }
        let Some(directive) = directives.get(&item.turn_id) else {
            continue;
        };
        let Some(evaluation) = evaluations.get(directive) else {
            continue;
        };
        let Some(result_id) = results.get(&item.turn_id) else {
            continue;
        };
        let status = status_for_completion(evaluation.completion.as_deref());
        let id = item
            .outcome
            .as_ref()
            .map(|value| value.result_id.clone())
            .unwrap_or_else(|| result_id.clone());
        item.outcome = Some(result_outcome(
            id,
            status,
            specs.get(directive),
            Some(evaluation),
            evidence.get(directive),
            review.get(directive),
        ));
        item.status = status;
    }
}

use vak_llm::{ContentBlock, Role};
use vak_session::{ActivityKind, ActivityStatus, EntryPayload, SessionLog};

fn sandbox_artifact_actions(
    execution_id: &str,
    path: &str,
    reviewable: bool,
) -> Vec<DeliveryAction> {
    let mut open_data = BTreeMap::new();
    open_data.insert("path".into(), path.into());
    let mut review_data = BTreeMap::new();
    review_data.insert("execution_id".into(), execution_id.into());
    let mut actions = vec![DeliveryAction {
        id: format!("open-{execution_id}-{path}"),
        label: "Open".into(),
        verb: "open_artifact".into(),
        data: open_data,
    }];
    if reviewable {
        actions.push(DeliveryAction {
            id: format!("review-{execution_id}"),
            label: "Review draft".into(),
            verb: "review_draft".into(),
            data: review_data,
        });
    }
    actions
}

/// One tool call as the timeline needs it: name, input, result text, and
/// whether the result was an error. Named because the inline tuple was wide
/// enough that a reader had to count commas to find the error flag.
type TurnTool = (String, String, serde_json::Value, Option<String>, bool);

/// The text a surface that cannot render cards natively — a chat channel, a
/// webhook, the inbox, a scheduled routine's summary — gets for a finished run.
///
/// A card emitted through an `emit_*_card` call is not in the model's final
/// text (that is only a line of narration), so delivering just that text drops
/// the card entirely. This takes the cards of the latest turn from the same
/// projection the desktop renders (so retries are superseded and nothing is
/// counted twice) and puts their deterministic text form ahead of the
/// narration.
pub(crate) fn text_with_run_cards(session: &SessionLog, narration: String) -> String {
    let session_id = session
        .header()
        .map(|header| header.session_id.clone())
        .unwrap_or_default();
    let timeline = snapshot(&session_id, session);
    let Some(turn) = timeline
        .items
        .iter()
        .rev()
        .find(|item| item.role == OutputRole::User)
        .map(|item| item.turn_id.clone())
    else {
        return narration;
    };
    let cards: Vec<&str> = timeline
        .items
        .iter()
        .filter(|item| {
            item.turn_id == turn
                && item.kind == OutputKind::Card
                && matches!(
                    item.content,
                    OutputContent::Structured { .. } | OutputContent::Adaptive { .. }
                )
        })
        .map(|item| item.fallback_text.trim())
        // Structured fallback text includes a JSON appendix for audit/export.
        // Chat channels receive the semantic card separately and should not
        // expose that appendix as the user-facing card.
        .map(|text| {
            text.split_once("\n\n```json")
                .map_or(text, |(body, _)| body)
        })
        .filter(|text| !text.is_empty())
        .collect();
    if cards.is_empty() {
        return narration;
    }
    let cards = cards.join("\n\n");
    match vak_delivery::visible_card_followup(&narration) {
        Some(followup) => format!("{cards}\n\n{followup}"),
        None => cards,
    }
}

/// One turn's answer as a page a person can keep (docs/design/98 §12): the
/// turn's items as the conversation drew them, its request as the title, and
/// as its text form its cards' text in the order shown, then the reply
/// (beside cards, only what the reply adds to them). `None` when the turn
/// has no answer.
pub(crate) fn answer_page(
    timeline: &OutputTimeline,
    turn: &str,
) -> Option<vak_delivery::page::Page> {
    let of_turn = || timeline.items.iter().filter(|item| item.turn_id == turn);
    let cards: Vec<&str> = of_turn()
        .filter(|item| {
            item.kind == OutputKind::Card
                && matches!(
                    item.content,
                    OutputContent::Structured { .. } | OutputContent::Adaptive { .. }
                )
        })
        .map(|item| item.fallback_text.trim())
        .map(|text| {
            text.split_once("\n\n```json")
                .map_or(text, |(body, _)| body)
        })
        .filter(|text| !text.is_empty())
        .collect();
    let reply = of_turn()
        .rev()
        .filter(|item| item.role == OutputRole::Assistant)
        .find_map(|item| match &item.content {
            OutputContent::Document { document } => Some(document.source_markdown.clone()),
            OutputContent::Outcome {
                document: Some(document),
                ..
            } => Some(document.source_markdown.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let reply = if cards.is_empty() {
        reply.trim().to_string()
    } else {
        vak_delivery::visible_card_followup(&reply).unwrap_or_default()
    };
    let body: Vec<&str> = cards
        .into_iter()
        .chain(std::iter::once(reply.as_str()))
        .filter(|part| !part.is_empty())
        .collect();
    if body.is_empty() {
        return None;
    }
    let title = of_turn()
        .find(|item| item.role == OutputRole::User)
        .and_then(|item| {
            item.fallback_text
                .lines()
                .find(|line| !line.trim().is_empty())
        })
        .map(|line| line.trim().to_string())
        .unwrap_or_default();
    Some(vak_delivery::page::Page::new(
        title,
        of_turn().cloned().collect(),
        timeline
            .answers
            .iter()
            .find(|outline| outline.turn_id == turn)
            .cloned(),
        body.join("\n\n"),
    ))
}

/// Keep the model's short narration separate from the structured cards.
/// Channel bridges have native card projections; including these same cards
/// in the narration made Discord/Slack show each result twice.
pub(crate) fn channel_answer(
    session: &SessionLog,
    narration: String,
) -> (String, Option<std::collections::BTreeMap<String, String>>) {
    let session_id = session
        .header()
        .map(|header| header.session_id.clone())
        .unwrap_or_default();
    let timeline = snapshot(&session_id, session);
    let Some(turn) = timeline
        .items
        .iter()
        .rev()
        .find(|item| item.role == OutputRole::User)
        .map(|item| item.turn_id.as_str())
    else {
        return (narration, None);
    };
    // Metadata travels with this reply, never through a mutable live handle
    // that may still be streaming or already belong to a continuation.
    let metadata = timeline.items.iter().rev().find_map(|item| {
        if item.turn_id != turn || item.role != OutputRole::Assistant {
            return None;
        }
        match &item.content {
            OutputContent::Document { document } => Some(document.metadata.clone()),
            _ => None,
        }
    });
    let has_card = timeline.items.iter().any(|item| {
        item.turn_id == turn
            && item.kind == OutputKind::Card
            && matches!(
                &item.content,
                OutputContent::Structured { .. } | OutputContent::Adaptive { .. }
            )
    });
    if !has_card {
        return (narration, metadata);
    }
    (
        vak_delivery::visible_card_followup(&narration).unwrap_or_default(),
        metadata,
    )
}

/// Typed cards from the latest turn, retained separately for channel-native
/// renderers. The text fallback remains alongside them for older consumers.
pub(crate) fn run_cards(session: &SessionLog) -> Vec<vak_delivery::StructuredOutput> {
    let session_id = session
        .header()
        .map(|header| header.session_id.clone())
        .unwrap_or_default();
    let timeline = snapshot(&session_id, session);
    let Some(turn) = timeline
        .items
        .iter()
        .rev()
        .find(|item| item.role == OutputRole::User)
        .map(|item| item.turn_id.as_str())
    else {
        return Vec::new();
    };
    timeline
        .items
        .iter()
        .filter(|item| item.turn_id == turn && item.kind == OutputKind::Card)
        .filter_map(|item| match &item.content {
            OutputContent::Structured { output } => Some(output.clone()),
            OutputContent::Adaptive { fallback_text, .. } => {
                vak_delivery::structured_outputs_from_text(fallback_text)
                    .into_iter()
                    .next()
            }
            _ => None,
        })
        .collect()
}

/// The turn the tool call `execution` belongs to and that turn's result,
/// which a draft put up for Review is bound to: what the turn answered
/// with, its words, else the card it showed, else the evaluation that
/// closed it. `None` when the call is not in the timeline, `Err` when its
/// turn has answered nothing yet. A turn that answered with a card and no
/// words still has a draft to review.
pub(crate) fn result_of_execution(
    timeline: &OutputTimeline,
    execution: &str,
) -> Option<Result<(String, String), ()>> {
    let turn_id = timeline.items.iter().find_map(|item| {
        item.provenance.as_ref().and_then(|value| {
            (value.tool_call_id.as_deref() == Some(execution)).then(|| item.turn_id.clone())
        })
    })?;
    let in_turn = || {
        timeline
            .items
            .iter()
            .rev()
            .filter(|item| item.turn_id == turn_id)
    };
    let result_id = in_turn()
        .find_map(|item| {
            (item.role == OutputRole::Assistant)
                .then(|| item.outcome.as_ref().map(|value| value.result_id.clone()))
                .flatten()
        })
        .or_else(|| {
            in_turn()
                .find(|item| item.kind == OutputKind::Card)
                .map(|item| item.id.clone())
        })
        .or_else(|| {
            in_turn()
                .find(|item| item.role == OutputRole::Assistant && item.kind == OutputKind::Outcome)
                .map(|item| item.id.clone())
        });
    Some(result_id.map(|result| (turn_id, result)).ok_or(()))
}

pub(crate) fn snapshot(session_id: &str, session: &SessionLog) -> OutputTimeline {
    snapshot_inner(
        session_id,
        session,
        &vak_delivery::built_in_skill_registry(),
        None,
    )
}

/// [`snapshot`] with the plugin-merged skill registry, so a plugin's kinds
/// in ```vak fences are read.
pub(crate) fn snapshot_with_skills(
    session_id: &str,
    session: &SessionLog,
    skills: &SkillRegistry,
) -> OutputTimeline {
    snapshot_inner(session_id, session, skills, None)
}

/// [`snapshot_with_skills`] with the effective presentation library, whose
/// active packs draw the kinds they accept.
pub(crate) fn snapshot_with_skills_and_library(
    session_id: &str,
    session: &SessionLog,
    skills: &SkillRegistry,
    library: &vak_presentation::PresentationLibrary,
) -> OutputTimeline {
    snapshot_inner(session_id, session, skills, Some(library))
}

/// Rehydrates sandbox-created artifacts into historical presentation views.
/// Sandbox events are persisted in a sidecar (because they are high-volume
/// telemetry), so they must be projected explicitly when a session is opened
/// after its live event bus is gone.
pub(crate) fn append_sandbox_artifacts(
    timeline: &mut OutputTimeline,
    home: &std::path::Path,
    session_id: &str,
) {
    let path = vak_config::scope::AgentScope::new(home).sandbox_executions(session_id);
    let events = crate::sandbox_records::events::<vak_tools::SandboxEvent>(&path);
    let records =
        crate::sandbox_records::load(&vak_config::scope::AgentScope::new(home).sandbox_records())
            .ok();
    append_sandbox_artifact_rows(timeline, home, session_id, events, records);
}

pub(crate) fn append_sandbox_artifacts_page(
    timeline: &mut OutputTimeline,
    home: &std::path::Path,
    session_id: &str,
    catalog: &vak_catalog::Catalog,
) -> Result<(), vak_catalog::CatalogError> {
    let calls: Vec<String> = timeline
        .items
        .iter()
        .filter_map(|item| item.provenance.as_ref()?.tool_call_id.clone())
        .collect();
    let scope = vak_config::scope::AgentScope::new(home);
    let decode = |error: serde_json::Error| vak_session::SessionError::Objects(error.to_string());
    let events = catalog
        .sandbox_rows(
            &scope.sandbox_executions(session_id),
            Some(session_id),
            session_id,
            &calls,
        )?
        .into_iter()
        .filter(|row| row.get("kind").and_then(serde_json::Value::as_str) != Some("Output"))
        .map(|row| serde_json::from_value(row).map_err(decode))
        .collect::<Result<Vec<vak_tools::SandboxEvent>, _>>()?;
    let records = catalog
        .sandbox_rows(&scope.sandbox_records(), None, session_id, &calls)?
        .into_iter()
        .map(|row| serde_json::from_value(row).map_err(decode))
        .collect::<Result<Vec<vak_sandbox::DurableRecord>, _>>()?;
    append_sandbox_artifact_rows(timeline, home, session_id, events, Some(records));
    Ok(())
}

fn append_sandbox_artifact_rows(
    timeline: &mut OutputTimeline,
    _home: &std::path::Path,
    session_id: &str,
    events: Vec<vak_tools::SandboxEvent>,
    records: Option<Vec<vak_sandbox::DurableRecord>>,
) {
    // A sandbox execution id is the brokered tool-call id. Resolve it back
    // to the durable turn before appending sidecar artifacts, so the result
    // stays one coherent turn on reconnect. The old synthetic
    // `sandbox-{execution_id}` turn forced clients to guess the association
    // from prose and file paths.
    let execution_context: HashMap<String, (String, String)> = timeline
        .items
        .iter()
        .filter_map(|item| {
            item.provenance
                .as_ref()?
                .tool_call_id
                .as_ref()
                .map(|id| (id.clone(), (item.turn_id.clone(), item.timestamp.clone())))
        })
        .collect();
    let result_by_turn: HashMap<String, ResultOutcome> = timeline
        .items
        .iter()
        .filter(|item| item.role != OutputRole::User)
        .filter_map(|item| {
            item.outcome
                .clone()
                .map(|outcome| (item.turn_id.clone(), outcome))
        })
        .collect();
    let reviewable_roots = events
        .iter()
        .filter_map(|event| match event {
            vak_tools::SandboxEvent::ExecutionStarted {
                execution_id,
                scratch_dir,
                ..
            } if std::path::Path::new(scratch_dir).is_dir() => {
                Some((execution_id.clone(), scratch_dir.clone()))
            }
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    // Unreadable records leave every draft's status unknown rather than
    // reporting a version the records may contradict.
    let drafts = records.map(|records| DraftVersions::new(records, session_id));
    for event in events {
        let vak_tools::SandboxEvent::ArtifactGenerated {
            execution_id,
            path,
            mime_type,
            size_bytes,
        } = event
        else {
            continue;
        };
        let id = format!("artifact-{execution_id}-{path}");
        if timeline.items.iter().any(|item| item.id == id) {
            continue;
        }
        let name = std::path::Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&path)
            .to_string();
        let (turn_id, timestamp) = execution_context
            .get(&execution_id)
            .cloned()
            .unwrap_or_else(|| {
                (
                    format!("sandbox-{execution_id}"),
                    "1970-01-01T00:00:00+00:00".into(),
                )
            });
        // Only an execution with drafts in the execution root holds a draft to
        // review; one that worked in the workspace already put its files
        // where they belong, and candidate export refuses it.
        let scratch = reviewable_roots
            .get(&execution_id)
            .filter(|root| draft_relative_path(&path, root).is_some());
        let reviewable = scratch.is_some();
        let status = match scratch {
            Some(root) => drafts
                .as_ref()
                .map(|drafts| drafts.status(&execution_id, &path, root)),
            None => Some(ArtifactStatus::InFolder),
        };
        // A successful write/edit is already projected from the durable tool
        // result. Its sandbox event is stronger evidence about the same file,
        // not a second artifact. Merge the sidecar metadata and actions so a
        // result presents one file and one Canvas entry point. A draft
        // accepted through Review is also recorded as a write of the folder
        // file by the call that drafted it: that is the draft's acceptance,
        // which its status already shows, so it is absorbed too.
        let same_file = |item: &OutputItem| {
            if item.kind != OutputKind::Artifact
                || item
                    .provenance
                    .as_ref()
                    .and_then(|p| p.tool_call_id.as_deref())
                    != Some(execution_id.as_str())
            {
                return false;
            }
            let OutputContent::Artifact { artifact } = &item.content else {
                return false;
            };
            artifact.path.as_deref().is_some_and(|existing_path| {
                let existing = std::path::Path::new(existing_path);
                let observed = std::path::Path::new(&path);
                existing == observed || existing.ends_with(observed) || observed.ends_with(existing)
            })
        };
        if let Some(first) = timeline.items.iter().position(&same_file) {
            let mut at = 0;
            timeline.items.retain(|item| {
                let keep = at <= first || !same_file(item);
                at += 1;
                keep
            });
            let existing = &mut timeline.items[first];
            existing.turn_id = turn_id.clone();
            existing.timestamp = timestamp;
            existing.outcome = result_by_turn.get(&turn_id).cloned();
            existing.actions = sandbox_artifact_actions(&execution_id, &path, reviewable);
            existing.fallback_text = format!("Generated artifact: {path}");
            if let OutputContent::Artifact { artifact } = &mut existing.content {
                artifact.path = Some(path.clone());
                artifact.media_type = Some(mime_type);
                artifact.description = None;
                artifact.size_bytes = Some(size_bytes);
                artifact.status = status;
            }
            continue;
        }
        timeline.items.push(OutputItem {
            placement: Default::default(),
            card_state: None,
            id,
            timestamp,
            turn_id: turn_id.clone(),
            role: OutputRole::Tool,
            kind: OutputKind::Artifact,
            status: OutputStatus::Succeeded,
            outcome: result_by_turn.get(&turn_id).cloned(),
            content: OutputContent::Artifact {
                artifact: ArtifactRef {
                    name,
                    path: Some(path.clone()),
                    media_type: Some(mime_type),
                    description: None,
                    size_bytes: Some(size_bytes),
                    status,
                },
            },
            provenance: Some(OutputProvenance {
                session_id: Some(session_id.to_string()),
                entry_id: None,
                tool_call_id: Some(execution_id.clone()),
                source: Some("sandbox_artifact".into()),
                presentation_id: None,
            }),
            actions: sandbox_artifact_actions(&execution_id, &path, reviewable),
            fallback_text: format!("Generated artifact: {path}"),
        });
    }
    deduplicate_file_artifacts_with_scratch(&mut timeline.items, &reviewable_roots);
}

/// The saved versions of each execution's draft in one conversation, in the
/// order the durable records hold them. Versions of one draft are
/// alternatives, so an acceptance settles every version saved before it and
/// a version saved afterwards starts a new round; undoing an acceptance
/// reopens its round. This is the rule Review applies
/// (`vak-client-ui/src/candidateVersions.ts`), so the card and Review agree.
struct DraftVersions {
    versions: HashMap<String, Vec<vak_sandbox::CandidateManifest>>,
    /// Per execution, the version index a live (not undone) acceptance
    /// settled, and whether a version was saved after it.
    accepted: HashMap<String, (usize, bool)>,
}

impl DraftVersions {
    fn new(records: Vec<vak_sandbox::DurableRecord>, session_id: &str) -> Self {
        use vak_sandbox::DurableRecord;
        let undone: HashSet<String> = records
            .iter()
            .filter_map(|record| match record {
                DurableRecord::PromotionUndo(undo) if undo.session_id == session_id => {
                    Some(undo.candidate_id.clone())
                }
                _ => None,
            })
            .collect();
        let mut owner: HashMap<String, (String, usize)> = HashMap::new();
        let mut versions: HashMap<String, Vec<vak_sandbox::CandidateManifest>> = HashMap::new();
        let mut accepted: HashMap<String, (usize, bool)> = HashMap::new();
        for record in records {
            match record {
                DurableRecord::Candidate(candidate) if candidate.session_id == session_id => {
                    let list = versions.entry(candidate.execution_id.clone()).or_default();
                    owner.insert(
                        candidate.candidate.candidate_id.clone(),
                        (candidate.execution_id.clone(), list.len()),
                    );
                    list.push(candidate.candidate);
                    if let Some(state) = accepted.get_mut(&candidate.execution_id) {
                        state.1 = true;
                    }
                }
                DurableRecord::Promotion(promotion)
                    if promotion.session_id == session_id
                        && !undone.contains(&promotion.candidate_id) =>
                {
                    if let Some((execution, index)) = owner.get(&promotion.candidate_id) {
                        accepted.insert(execution.clone(), (*index, false));
                    }
                }
                _ => {}
            }
        }
        Self { versions, accepted }
    }

    /// The status of the file at `path`, produced by `execution_id` in the
    /// scratch directory `scratch`.
    fn status(&self, execution_id: &str, path: &str, scratch: &str) -> ArtifactStatus {
        let versions = self
            .versions
            .get(execution_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let saved_as = |index: usize| {
            let relative = draft_relative_path(path, scratch)?;
            versions
                .get(index)?
                .files
                .iter()
                .find(|file| std::path::Path::new(&file.path) == relative)
                .map(|file| vak_delivery::VersionFile {
                    version_id: versions[index].candidate_id.clone(),
                    path: file.path.clone(),
                })
        };
        match self.accepted.get(execution_id) {
            Some(&(index, false)) => ArtifactStatus::Accepted {
                version: version_number(index),
                saved_as: saved_as(index),
            },
            _ if versions.is_empty() => ArtifactStatus::Draft {
                version: 1,
                saved_as: None,
            },
            _ => ArtifactStatus::Draft {
                version: version_number(versions.len() - 1),
                saved_as: saved_as(versions.len() - 1),
            },
        }
    }
}

fn version_number(index: usize) -> u32 {
    u32::try_from(index).map_or(u32::MAX, |index| index.saturating_add(1))
}

/// `path` (absolute, or a draft name `.vak/scratch/<agent>/<execution>/…`)
/// relative to the execution directory it was written in, which is how a
/// saved version names it. The directory is in the runtime root and ends in
/// the same `<agent>/<execution>` as the draft's name.
fn draft_relative_path<'a>(path: &'a str, scratch: &str) -> Option<&'a std::path::Path> {
    let artifact = std::path::Path::new(path);
    let scratch = std::path::Path::new(scratch);
    if artifact.is_absolute() {
        return artifact.strip_prefix(scratch).ok();
    }
    let execution = scratch.file_name()?;
    let agent = scratch.parent()?.file_name()?;
    artifact
        .strip_prefix(vak_config::scope::SCRATCH_DIR)
        .ok()?
        .strip_prefix(std::path::Path::new(agent).join(execution))
        .ok()
}

/// Repeated successful writes to one file within a turn are revisions of the
/// same deliverable. Keep the newest observed artifact and its actions, while
/// leaving identically named files in other turns or directories distinct.
fn deduplicate_file_artifacts(items: &mut Vec<OutputItem>) {
    deduplicate_file_artifacts_with_scratch(items, &HashMap::new());
}

fn deduplicate_file_artifacts_with_scratch(
    items: &mut Vec<OutputItem>,
    scratch_roots: &HashMap<String, String>,
) {
    let mut seen = HashSet::new();
    let mut latest_first = Vec::with_capacity(items.len());
    for item in items.drain(..).rev() {
        let duplicate = if let OutputContent::Artifact { artifact } = &item.content {
            artifact.path.as_ref().is_some_and(|path| {
                let execution_id = item
                    .provenance
                    .as_ref()
                    .and_then(|provenance| provenance.tool_call_id.as_ref());
                let identity = execution_id
                    .and_then(|id| scratch_roots.get(id).map(|root| (id, root)))
                    .and_then(|(_, root)| draft_relative_path(path, root))
                    .map(|relative| format!("scratch:{}", relative.display()))
                    .unwrap_or_else(|| path.clone());
                !seen.insert((item.turn_id.clone(), identity))
            })
        } else {
            false
        };
        if !duplicate {
            latest_first.push(item);
        }
    }
    latest_first.reverse();
    *items = latest_first;
}

/// Returns a concise, transport-neutral artifact list for channel delivery.
/// The browser can resolve richer previews, while text surfaces still need an
/// explicit record that the generated files exist and where they live.
pub(crate) fn sandbox_artifact_markdown(
    home: &std::path::Path,
    session_id: &str,
) -> Option<String> {
    let path = vak_config::scope::AgentScope::new(home).sandbox_executions(session_id);
    let mut rows = Vec::new();
    for event in crate::sandbox_records::events::<vak_tools::SandboxEvent>(&path) {
        let vak_tools::SandboxEvent::ArtifactGenerated {
            path,
            mime_type,
            size_bytes,
            ..
        } = event
        else {
            continue;
        };
        let row = format!("- `{path}` ({mime_type}, {size_bytes} bytes)");
        if !rows.contains(&row) {
            rows.push(row);
        }
    }
    (!rows.is_empty()).then(|| format!("\n\nGenerated artifacts\n\n{}", rows.join("\n")))
}

fn has_user_request_content(content: &[ContentBlock]) -> bool {
    content.iter().any(|block| match block {
        ContentBlock::Text { text } => !clean_scaffolding(text).is_empty(),
        ContentBlock::Image { .. } => true,
        _ => false,
    })
}

fn snapshot_inner(
    session_id: &str,
    session: &SessionLog,
    skills: &SkillRegistry,
    adaptive_library: Option<&vak_presentation::PresentationLibrary>,
) -> OutputTimeline {
    let chain = session.chain_to_root();
    // Presentations are ledger entries (docs/design/68-context-engine.md
    // §10): the card a tool call displayed is read from its own written
    // entry, keyed by `tool_use_id` — never rebuilt from the call's
    // arguments at display time.
    // The map's value keeps the Presentation entry's OWN id alongside the
    // record: `presentation_id` on the projected item is this id, not the
    // id of the message entry the tool call rode in on (docs/design/68 §10:
    // feedback/selection key on the ledger fact, i.e. this entry).
    let presentation_by_tool_use_id: HashMap<
        String,
        (String, &vak_session::types::PresentationRecord),
    > = session
        .presentations()
        .into_iter()
        .filter_map(|(entry_id, record)| match &record.source {
            vak_session::types::PresentationSource::ToolCall { tool_use_id } => {
                Some((tool_use_id.clone(), (entry_id, record)))
            }
            vak_session::types::PresentationSource::Fence { .. }
            | vak_session::types::PresentationSource::Delegated { .. }
            | vak_session::types::PresentationSource::Answer { .. }
            | vak_session::types::PresentationSource::Scheduled { .. } => None,
        })
        .collect();
    // The cards a scheduled run showed, by the delivered turn they belong to
    // (docs/design/94 §4).
    let mut scheduled_cards: HashMap<
        String,
        Vec<(String, &vak_session::types::PresentationRecord)>,
    > = HashMap::new();
    for (entry_id, record) in session.presentations() {
        if matches!(
            record.source,
            vak_session::types::PresentationSource::Scheduled { .. }
        ) {
            scheduled_cards
                .entry(record.turn_id.clone())
                .or_default()
                .push((entry_id, record));
        }
    }
    // A delegated call (`task`) may carry several cards: the ones its worker
    // showed, recorded in this ledger when the worker ended.
    let mut delegated_by_tool_use_id: HashMap<
        String,
        Vec<(String, &vak_session::types::PresentationRecord)>,
    > = HashMap::new();
    // An `answer` is the turn's whole document: the latest one a turn gave
    // replaces any earlier one.
    let mut latest_answer: HashMap<String, String> = HashMap::new();
    for (_, record) in session.presentations() {
        if let vak_session::types::PresentationSource::Answer { tool_use_id, .. } = &record.source {
            latest_answer.insert(record.turn_id.clone(), tool_use_id.clone());
        }
    }
    for (entry_id, record) in session.presentations() {
        if let vak_session::types::PresentationSource::Answer { tool_use_id, .. } = &record.source
            && latest_answer.get(&record.turn_id) != Some(tool_use_id)
        {
            continue;
        }
        // An `answer` call carries its blocks the same way, in block order.
        if let vak_session::types::PresentationSource::Delegated { tool_use_id, .. }
        | vak_session::types::PresentationSource::Answer { tool_use_id, .. } = &record.source
        {
            delegated_by_tool_use_id
                .entry(tool_use_id.clone())
                .or_default()
                .push((entry_id, record));
        }
    }
    let mut tool_results: HashMap<String, (String, bool)> = HashMap::new();
    let mut tool_inputs: HashMap<String, (String, serde_json::Value)> = HashMap::new();
    let mut turn_outcomes: HashMap<String, vak_intent::OutcomeSpec> = HashMap::new();
    let mut turn_evaluations: HashMap<String, TurnEvaluation> = HashMap::new();
    let mut turn_evidence_state: HashMap<String, String> = HashMap::new();
    let mut turn_human_review: HashMap<String, String> = HashMap::new();
    let mut turn_review_verdict: HashMap<String, String> = HashMap::new();
    let mut selected_presentation: Option<(String, u64)> = None;
    let mut successful_runs = std::collections::HashSet::new();
    let mut assistant_tool_context: HashMap<String, TurnTool> = HashMap::new();
    let mut pending_tool_context: Option<TurnTool> = None;
    // Runtime validation and display use the same accepted replacement view.
    let retired_presentations = session.replaced_presentation_ids();
    let mut seen_cards: std::collections::HashSet<String> = std::collections::HashSet::new();
    for entry in &chain {
        match &entry.payload {
            EntryPayload::Message(record) => {
                // What the user wrote versus what the runtime authored is a
                // typed fact on the record (`vak_intent::control`), not
                // something to re-derive from the text.
                let control = record.control_kind();
                if record.message.role == Role::User
                    && control.is_none()
                    && has_user_request_content(&record.message.content)
                {
                    pending_tool_context = None;
                }
                for block in &record.message.content {
                    match block {
                        ContentBlock::ToolUse { id, name, input } => {
                            tool_inputs.insert(id.clone(), (name.clone(), input.clone()));
                        }
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        } => {
                            tool_results.insert(tool_use_id.clone(), (content.clone(), *is_error));
                            if let Some((name, input)) = tool_inputs.get(tool_use_id) {
                                let context = (
                                    tool_use_id.clone(),
                                    name.clone(),
                                    input.clone(),
                                    Some(content.clone()),
                                    *is_error,
                                );
                                pending_tool_context = Some(context);
                            }
                        }
                        _ => {}
                    }
                }
                if record.message.role == Role::Assistant
                    && record
                        .message
                        .content
                        .iter()
                        .any(|block| matches!(block, ContentBlock::Text { .. }))
                    && let Some(context) = pending_tool_context.take()
                {
                    assistant_tool_context.insert(entry.id.clone(), context);
                }
            }
            EntryPayload::Intent(record) => {
                if let Some(outcome) = &record.outcome {
                    // The intent names its turn (`Entry::at_turn`), the one
                    // whose directive follows it, so no position is guessed.
                    if let Some(turn) = &entry.at_turn {
                        turn_outcomes.insert(turn.clone(), outcome.clone());
                    }
                }
            }
            EntryPayload::Activity(activity)
                if activity.kind == ActivityKind::Diagnostic
                    && activity.label == "Outcome evaluation" =>
            {
                if let Some(turn) = entry.at_turn.clone() {
                    turn_evaluations.insert(turn.clone(), TurnEvaluation::from_activity(activity));
                    if let Some(evidence_state) = activity.data.get("evidence_state") {
                        turn_evidence_state.insert(turn.clone(), evidence_state.clone());
                    }
                    if let Some(review) = activity.data.get("human_review") {
                        turn_human_review.insert(turn.clone(), review.clone());
                    }
                }
            }
            EntryPayload::Activity(activity)
                if activity.kind == ActivityKind::PresentationSelection =>
            {
                if let (Some(spec_id), Some(revision)) = (
                    activity.data.get("spec_id"),
                    activity
                        .data
                        .get("revision")
                        .and_then(|value| value.parse().ok()),
                ) {
                    selected_presentation = Some((spec_id.clone(), revision));
                }
            }
            EntryPayload::Activity(activity)
                if activity.kind == ActivityKind::Diagnostic
                    && activity.label == "Outcome review" =>
            {
                if let Some(turn) = entry.at_turn.clone()
                    && let Some(verdict) = activity.data.get("verdict")
                {
                    turn_review_verdict.insert(turn, verdict.clone());
                }
            }
            EntryPayload::Activity(activity)
                if activity.kind == ActivityKind::Run
                    && activity.status == ActivityStatus::Succeeded =>
            {
                if let Some(turn) = entry.at_turn.clone() {
                    successful_runs.insert(turn);
                }
            }
            _ => {}
        }
    }

    let mut timeline = OutputTimeline::empty(session_id);
    timeline.goal = session.goal_state();
    let mut turn = 0usize;
    let mut current_turn = String::new();
    let mut turn_numbers: HashMap<String, usize> = HashMap::new();
    // An answer the runtime sent back for a redo (a `retries_answer` nudge
    // followed it) is an internal draft, not something to show: the user sees
    // the redone answer, never both. `last_assistant_entry` is the answer the
    // next such nudge would be rejecting.
    let mut last_assistant_entry: Option<String> = None;
    let mut rejected_drafts: std::collections::HashSet<String> = std::collections::HashSet::new();
    // A plan check that follows an answer asks only for what the plan still
    // lacks (docs/design/86-model-plans-runtime-checks.md §2.2); the text
    // written in reply to it is addressed to the runtime, so the person
    // keeps the answer they were given and sees only the cards and files
    // the reply adds.
    let mut answered_in_turn = false;
    let mut replying_to_runtime = false;
    let mut runtime_replies: std::collections::HashSet<String> = std::collections::HashSet::new();
    let written = written_files(&chain);
    for entry in chain {
        match &entry.payload {
            EntryPayload::Message(record) => {
                let control = record.control_kind();
                // Card de-duplication and repair state are consumed while
                // projecting items below, so reset/arm them in this same
                // traversal. Updating them in the earlier indexing pass
                // leaves the output pass's `seen_cards` populated across
                // user turns and hides an identical card on a later turn.
                if record.message.role == Role::User
                    && control.is_none()
                    && has_user_request_content(&record.message.content)
                {
                    seen_cards.clear();
                    answered_in_turn = false;
                    replying_to_runtime = false;
                }
                if control == Some(vak_intent::control::ControlKind::PlanCheck) {
                    replying_to_runtime = answered_in_turn;
                }
                if control.is_some_and(|kind| kind.discards_assistant_draft())
                    && let Some(draft) = last_assistant_entry.take()
                {
                    rejected_drafts.insert(draft);
                }
                if record.message.role == Role::Assistant
                    && record.message.content.iter().any(|block| {
                        matches!(block, ContentBlock::Text { text } if !text.trim().is_empty())
                    })
                {
                    last_assistant_entry = Some(entry.id.clone());
                    if replying_to_runtime {
                        runtime_replies.insert(entry.id.clone());
                        replying_to_runtime = false;
                    } else {
                        answered_in_turn = true;
                    }
                }
                if record.message.role == Role::User
                    && control.is_none()
                    && has_user_request_content(&record.message.content)
                {
                    turn += 1;
                    current_turn = entry.id.clone();
                    turn_numbers.insert(entry.id.clone(), turn);
                }
                let turn_id = format!("turn-{turn}");
                let scheduled = record
                    .meta
                    .as_ref()
                    .and_then(|meta| meta.scheduled.as_ref());
                if record.message.role == Role::Assistant
                    && scheduled.is_some()
                    && let Some(cards) = entry
                        .at_turn
                        .as_ref()
                        .and_then(|at| scheduled_cards.get(at))
                {
                    for (index, (presentation, card)) in cards.iter().enumerate() {
                        let output = vak_delivery::StructuredOutput {
                            semantic_type: card.semantic_type.clone(),
                            block: card.block.clone().unwrap_or_else(|| {
                                if vak_delivery::is_form(&card.semantic_type) {
                                    card.semantic_type.clone()
                                } else {
                                    String::new()
                                }
                            }),
                            schema_version: u16::try_from(card.schema_version).unwrap_or(u16::MAX),
                            skill_id: card.skill_id.clone(),
                            skill_version: card.skill_version.clone(),
                            payload: card.payload.clone(),
                        };
                        timeline.items.push(OutputItem {
                            placement: Default::default(),
                            card_state: None,
                            id: format!("{}-scheduled-{index}", entry.id),
                            timestamp: entry.ts.to_rfc3339(),
                            turn_id: turn_id.clone(),
                            role: OutputRole::Tool,
                            kind: OutputKind::Card,
                            status: OutputStatus::Succeeded,
                            outcome: None,
                            fallback_text: structured_markdown(&output),
                            content: OutputContent::Structured { output },
                            provenance: Some(OutputProvenance {
                                session_id: Some(session_id.into()),
                                entry_id: Some(entry.id.clone()),
                                tool_call_id: None,
                                source: Some("scheduled".into()),
                                presentation_id: Some(presentation.clone()),
                            }),
                            actions: Vec::new(),
                        });
                    }
                }
                for (index, block) in record.message.content.iter().enumerate() {
                    match block {
                        ContentBlock::Text { text } if !text.trim().is_empty() => {
                            if control.is_some() {
                                continue;
                            }
                            let assistant = record.message.role == Role::Assistant;
                            // A delivered answer whose cards are drawn above
                            // shows its narration, not their text again.
                            let text = scheduled
                                .filter(|_| assistant)
                                .and_then(|scheduled| scheduled.narration.as_deref())
                                .unwrap_or(text);
                            let cleaned_text_storage = clean_scaffolding(text);
                            if cleaned_text_storage.is_empty() {
                                continue;
                            }
                            let text = &cleaned_text_storage;
                            let candidates = if assistant {
                                structured_outputs_from_text_with(text, skills)
                            } else {
                                Vec::new()
                            };
                            let assistant_tool_call_id = assistant_tool_context
                                .get(&entry.id)
                                .map(|(id, ..)| id.clone());
                            let projected_outcome = turn_outcomes.get(&current_turn).cloned();
                            // An activation is the user's standing choice for this
                            // scope. Use it automatically for the first compatible
                            // typed result; explicit session selection still wins.
                            let selected_for_output = selected_presentation.clone().or_else(|| {
                                adaptive_library.and_then(|library| {
                                    let workspace_owner = session
                                        .header()
                                        .map(|header| {
                                            header.contract_cwd().to_string_lossy().into_owned()
                                        })
                                        .unwrap_or_else(|| "workspace".into());
                                    candidates.iter().find_map(|candidate| {
                                        library
                                            .select_preferred(
                                                &candidate.semantic_type,
                                                "user",
                                                &workspace_owner,
                                            )
                                            .map(|stored| {
                                                (stored.spec.id.clone(), stored.spec.revision)
                                            })
                                    })
                                })
                            });
                            let output_status = status_for_completion(
                                turn_evaluations
                                    .get(&current_turn)
                                    .and_then(|value| value.completion.as_deref()),
                            );
                            timeline.items.push(OutputItem {
                                placement: Default::default(),
                                card_state: None,
                                id: format!("{}-text-{index}", entry.id),
                                timestamp: entry.ts.to_rfc3339(),
                                turn_id: turn_id.clone(),
                                role: if assistant {
                                    OutputRole::Assistant
                                } else {
                                    OutputRole::User
                                },
                                kind: if assistant {
                                    OutputKind::Outcome
                                } else {
                                    OutputKind::Message
                                },
                                status: output_status,
                                content: OutputContent::Document {
                                    document: {
                                        let mut document = compile_markdown(text.clone());
                                        if let Some(library) = adaptive_library {
                                            let available = candidates
                                                .iter()
                                                .filter(|candidate| {
                                                    library.definitions().any(|stored| {
                                                        stored.spec.accepts.iter().any(|kind| {
                                                            kind == &candidate.semantic_type
                                                        })
                                                    })
                                                })
                                                .count();
                                            if available > 0 {
                                                document.metadata.insert(
                                                    "adaptive_definitions_available".into(),
                                                    available.to_string(),
                                                );
                                            }
                                            if let Some((spec_id, revision)) =
                                                selected_for_output.as_ref()
                                                && library.definitions().any(|stored| {
                                                    stored.spec.id == *spec_id
                                                        && stored.spec.revision == *revision
                                                })
                                            {
                                                document.metadata.insert(
                                                    "adaptive_selected_spec".into(),
                                                    format!("{spec_id}@{revision}"),
                                                );
                                            }
                                        }
                                        if let Some(scheduled) = record
                                            .meta
                                            .as_ref()
                                            .and_then(|meta| meta.scheduled.as_ref())
                                        {
                                            for (key, value) in [
                                                ("scheduled_trigger", scheduled.trigger.clone()),
                                                ("scheduled_name", scheduled.name.clone()),
                                                ("scheduled_run", scheduled.run.clone()),
                                                ("scheduled_failed", scheduled.failed.to_string()),
                                            ] {
                                                document.metadata.insert(key.into(), value);
                                            }
                                            if let Some(run_session) = &scheduled.run_session {
                                                document.metadata.insert(
                                                    "scheduled_run_session".into(),
                                                    run_session.clone(),
                                                );
                                            }
                                        }
                                        if let Some(outcome) = projected_outcome.as_ref() {
                                            document.metadata.insert(
                                                "outcome_objective".into(),
                                                outcome.objective.clone(),
                                            );
                                            document.metadata.insert(
                                                "outcome_revision".into(),
                                                outcome.revision.to_string(),
                                            );
                                            document.metadata.insert(
                                                "outcome_requirements".into(),
                                                outcome
                                                    .requirements
                                                    .iter()
                                                    .map(|requirement| requirement.id.as_str())
                                                    .collect::<Vec<_>>()
                                                    .join(","),
                                            );
                                        }
                                        if let Some(evaluation) =
                                            turn_evaluations.get(&current_turn)
                                        {
                                            document.metadata.insert(
                                                "outcome_status".into(),
                                                evaluation.status.clone(),
                                            );
                                            document.metadata.insert(
                                                "outcome_evaluation".into(),
                                                evaluation.evaluation.clone(),
                                            );
                                            document.metadata.insert(
                                                "outcome_evidence_receipts".into(),
                                                evaluation.receipts.join(","),
                                            );
                                            if !evaluation.requirement_ids.is_empty() {
                                                document.metadata.insert(
                                                    "outcome_requirements".into(),
                                                    evaluation.requirement_ids.join(","),
                                                );
                                            }
                                            if let Some(completion) = &evaluation.completion {
                                                document.metadata.insert(
                                                    "outcome_completion".into(),
                                                    completion.clone(),
                                                );
                                            }
                                        }
                                        if let Some(evidence_state) =
                                            turn_evidence_state.get(&current_turn)
                                        {
                                            document.metadata.insert(
                                                "outcome_evidence_state".into(),
                                                evidence_state.clone(),
                                            );
                                        }
                                        if let Some(review) = turn_human_review.get(&current_turn) {
                                            document.metadata.insert(
                                                "outcome_human_review".into(),
                                                review.clone(),
                                            );
                                        }
                                        if let Some(verdict) =
                                            turn_review_verdict.get(&current_turn)
                                        {
                                            document.metadata.insert(
                                                "outcome_review_verdict".into(),
                                                verdict.clone(),
                                            );
                                        }
                                        document
                                    },
                                },
                                outcome: Some(result_outcome(
                                    format!("{}-text-{index}", entry.id),
                                    output_status,
                                    projected_outcome.as_ref(),
                                    turn_evaluations.get(&current_turn),
                                    turn_evidence_state.get(&current_turn),
                                    turn_human_review.get(&current_turn),
                                )),
                                provenance: Some(OutputProvenance {
                                    session_id: Some(session_id.into()),
                                    entry_id: Some(entry.id.clone()),
                                    tool_call_id: assistant_tool_call_id,
                                    source: Some("session_ledger".into()),
                                    presentation_id: None,
                                }),
                                actions: Vec::new(),
                                fallback_text: text.clone(),
                            });
                            if let (Some(library), Some((spec_id, revision))) =
                                (adaptive_library, selected_for_output.as_ref())
                                && let Some(stored) = library.definitions().find(|stored| {
                                    stored.spec.id == *spec_id && stored.spec.revision == *revision
                                })
                                && let Some(candidate) = candidates.iter().find(|candidate| {
                                    stored
                                        .spec
                                        .accepts
                                        .iter()
                                        .any(|kind| kind == &candidate.semantic_type)
                                })
                                && let Some(content) = OutputContent::from_compiled_adaptive(
                                    vak_presentation::compile(
                                        &stored.spec,
                                        &vak_presentation::CompileInput {
                                            semantic_type: candidate.semantic_type.clone(),
                                            payload: candidate.payload.clone(),
                                            fallback_text: text.clone(),
                                        },
                                    ),
                                    text.clone(),
                                )
                            {
                                timeline.items.push(OutputItem {
                                    placement: Default::default(),
                                    card_state: None,
                                    id: format!("{}-adaptive", entry.id),
                                    timestamp: entry.ts.to_rfc3339(),
                                    turn_id: turn_id.clone(),
                                    role: OutputRole::Assistant,
                                    kind: OutputKind::Outcome,
                                    status: output_status,
                                    outcome: None,
                                    content,
                                    provenance: Some(OutputProvenance {
                                        session_id: Some(session_id.into()),
                                        entry_id: Some(entry.id.clone()),
                                        tool_call_id: None,
                                        source: Some("adaptive_library".into()),
                                        presentation_id: None,
                                    }),
                                    actions: Vec::new(),
                                    fallback_text: text.clone(),
                                });
                            }
                        }
                        ContentBlock::ToolUse { id, name, .. } => {
                            let result = tool_results.get(id);
                            let failed = result.is_some_and(|(_, failed)| *failed);
                            let recovered = failed && successful_runs.contains(&current_turn);
                            let detail = result.map(|(content, _)| content.clone());
                            timeline.items.push(OutputItem {
                                placement: Default::default(),
                                card_state: None,
                                id: id.clone(),
                                timestamp: entry.ts.to_rfc3339(),
                                turn_id: turn_id.clone(),
                                role: OutputRole::Tool,
                                kind: if failed && !recovered {
                                    OutputKind::Error
                                } else {
                                    OutputKind::Progress
                                },
                                outcome: None,
                                status: if failed {
                                    OutputStatus::Failed
                                } else {
                                    OutputStatus::Succeeded
                                },
                                content: if failed && !recovered {
                                    OutputContent::Error {
                                        message: detail
                                            .clone()
                                            .unwrap_or_else(|| "Tool failed".into()),
                                        source: Some(name.clone()),
                                        retryable: false,
                                    }
                                } else {
                                    OutputContent::Progress {
                                        label: if recovered {
                                            format!("Recovered {name}")
                                        } else {
                                            name.clone()
                                        },
                                        detail: detail.clone(),
                                        percent: None,
                                    }
                                },
                                provenance: Some(OutputProvenance {
                                    session_id: Some(session_id.into()),
                                    entry_id: Some(entry.id.clone()),
                                    tool_call_id: Some(id.clone()),
                                    source: Some("tool_call".into()),
                                    presentation_id: None,
                                }),
                                actions: Vec::new(),
                                fallback_text: detail
                                    .clone()
                                    .unwrap_or_else(|| format!("{name} completed")),
                            });
                            // A tool renders richly one of two ways, neither of
                            // which names the tool: it self-declares a
                            // `semantic_type` in its own result (fenced or bare —
                            // `structured_outputs_from_text`), or — since most
                            // tools are third-party and cannot be asked to adopt
                            // our envelope — a registered `ResultAdapter`
                            // recognizes its specific, known response shape
                            // (`built_in_adapters`). Either way the candidate
                            // still has to pass `SkillRegistry::validate` before
                            // anything renders from it, and the tool's own
                            // stored result is read, never rewritten.
                            if !failed {
                                // A card call's card is read from its own
                                // `Presentation` ledger entry
                                // (docs/design/68-context-engine.md §10),
                                // keyed by this call's `tool_use_id` —
                                // written once, at validation, and never
                                // rebuilt from the call's arguments here.
                                // Only a tool that `presents_cards()` writes
                                // a `ToolCall` presentation (the agent's
                                // `show_card`), so the entry is the fact.
                                let has_card_entry = presentation_by_tool_use_id.contains_key(id);
                                let recorded = |(entry_id, record): &(
                                    String,
                                    &vak_session::types::PresentationRecord,
                                )| {
                                    (
                                        vak_delivery::StructuredOutput {
                                            semantic_type: record.semantic_type.clone(),
                                            block: record.block.clone().unwrap_or_else(|| {
                                                if vak_delivery::is_form(&record.semantic_type) {
                                                    record.semantic_type.clone()
                                                } else {
                                                    String::new()
                                                }
                                            }),
                                            schema_version: u16::try_from(record.schema_version)
                                                .unwrap_or(u16::MAX),
                                            skill_id: record.skill_id.clone(),
                                            skill_version: record.skill_version.clone(),
                                            payload: record.payload.clone(),
                                        },
                                        Some(entry_id.clone()),
                                    )
                                };
                                // Each output with the Presentation entry it
                                // was written as, when it has one.
                                let outputs: Vec<(vak_delivery::StructuredOutput, Option<String>)> =
                                    if has_card_entry {
                                        presentation_by_tool_use_id
                                            .get(id)
                                            .map(|(entry_id, record)| {
                                                recorded(&(entry_id.clone(), *record))
                                            })
                                            .into_iter()
                                            .collect()
                                    } else {
                                        delegated_by_tool_use_id
                                            .get(id)
                                            .map(|cards| cards.iter().map(recorded).collect())
                                            .unwrap_or_default()
                                    };
                                for (structured_index, (output, presentation_entry_id)) in
                                    outputs.into_iter().enumerate()
                                {
                                    if presentation_entry_id
                                        .as_ref()
                                        .is_some_and(|id| retired_presentations.contains(id))
                                    {
                                        continue;
                                    }
                                    // The same card emitted twice in one answer is one
                                    // card, armed retry or not: identical content has
                                    // nothing to supersede and nothing to add.
                                    if has_card_entry
                                        && !seen_cards.insert(format!(
                                            "{}:{}",
                                            serde_json::to_string(&output).unwrap_or_default(),
                                            presentation_by_tool_use_id
                                                .get(id)
                                                .and_then(|(_, record)| record
                                                    .data_call_id
                                                    .as_deref())
                                                .unwrap_or_default()
                                        ))
                                    {
                                        continue;
                                    }
                                    let item_id = format!("{id}-structured-{structured_index}");
                                    let fallback_text = structured_markdown(&output);
                                    let workspace_owner = session
                                        .header()
                                        .map(|header| {
                                            header.contract_cwd().to_string_lossy().into_owned()
                                        })
                                        .unwrap_or_else(|| "workspace".into());
                                    let selected = adaptive_library.and_then(|library| {
                                        selected_presentation
                                            .as_ref()
                                            .and_then(|(spec_id, revision)| {
                                                library.get(spec_id, *revision)
                                            })
                                            .filter(|stored| {
                                                stored.spec.accepts.contains(&output.semantic_type)
                                            })
                                            .or_else(|| {
                                                library.select_preferred(
                                                    &output.semantic_type,
                                                    "user",
                                                    &workspace_owner,
                                                )
                                            })
                                    });
                                    let adaptive = selected.and_then(|stored| {
                                        OutputContent::from_compiled_adaptive(
                                            vak_presentation::compile(
                                                &stored.spec,
                                                &vak_presentation::CompileInput {
                                                    semantic_type: output.semantic_type.clone(),
                                                    payload: output.payload.clone(),
                                                    fallback_text: fallback_text.clone(),
                                                },
                                            ),
                                            fallback_text.clone(),
                                        )
                                    });
                                    timeline.items.push(OutputItem {
                                        placement: Default::default(),
                                        card_state: None,
                                        id: item_id,
                                        timestamp: entry.ts.to_rfc3339(),
                                        turn_id: turn_id.clone(),
                                        role: OutputRole::Tool,
                                        kind: OutputKind::Card,
                                        status: OutputStatus::Succeeded,
                                        outcome: None,
                                        fallback_text,
                                        content: adaptive
                                            .unwrap_or(OutputContent::Structured { output }),
                                        provenance: Some(OutputProvenance {
                                            session_id: Some(session_id.into()),
                                            entry_id: Some(entry.id.clone()),
                                            tool_call_id: Some(id.clone()),
                                            source: Some(name.clone()),
                                            presentation_id: presentation_entry_id,
                                        }),
                                        actions: Vec::new(),
                                    });
                                }
                            }
                            for (n, artifact) in written
                                .get(id.as_str())
                                .into_iter()
                                .flatten()
                                .filter(|_| !failed)
                                .map(|path| artifact_at(path))
                                .enumerate()
                            {
                                let mut data = BTreeMap::new();
                                if let Some(path) = &artifact.path {
                                    data.insert("path".into(), path.clone());
                                }
                                timeline.items.push(OutputItem {
                                    placement: Default::default(),
                                    card_state: None,
                                    id: if n == 0 {
                                        format!("{id}-artifact")
                                    } else {
                                        format!("{id}-artifact-{n}")
                                    },
                                    timestamp: entry.ts.to_rfc3339(),
                                    turn_id: turn_id.clone(),
                                    role: OutputRole::Tool,
                                    kind: OutputKind::Artifact,
                                    status: OutputStatus::Succeeded,
                                    outcome: None,
                                    content: OutputContent::Artifact {
                                        artifact: artifact.clone(),
                                    },
                                    provenance: Some(OutputProvenance {
                                        session_id: Some(session_id.into()),
                                        entry_id: Some(entry.id.clone()),
                                        tool_call_id: Some(id.clone()),
                                        source: Some(name.clone()),
                                        presentation_id: None,
                                    }),
                                    actions: vec![DeliveryAction {
                                        id: format!("open-{id}"),
                                        label: "Open".into(),
                                        verb: "open_artifact".into(),
                                        data,
                                    }],
                                    fallback_text: artifact
                                        .path
                                        .clone()
                                        .unwrap_or_else(|| artifact.name.clone()),
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
            EntryPayload::Activity(activity) if is_user_facing_activity(activity) => {
                let number = entry
                    .at_turn
                    .as_ref()
                    .and_then(|id| turn_numbers.get(id))
                    .copied()
                    .unwrap_or(turn);
                timeline.items.push(activity_item(
                    session_id,
                    &entry.id,
                    &entry.ts.to_rfc3339(),
                    &format!("turn-{number}"),
                    entry.at_turn.as_deref(),
                    activity,
                ));
            }
            // Goal changes reach the client through `timeline.goal_state`;
            // a per-update progress row is runtime bookkeeping.
            EntryPayload::GoalUpdate(_) => {}
            _ => {}
        }
    }
    let mut positions = HashMap::new();
    let mut deduplicated = Vec::with_capacity(timeline.items.len());
    for item in timeline.items.drain(..) {
        if let Some(index) = positions.get(&item.id).copied() {
            deduplicated[index] = item;
        } else {
            positions.insert(item.id.clone(), deduplicated.len());
            deduplicated.push(item);
        }
    }
    if !rejected_drafts.is_empty() {
        deduplicated.retain(|item| {
            !(item.role == OutputRole::Assistant
                && item
                    .provenance
                    .as_ref()
                    .and_then(|p| p.entry_id.as_ref())
                    .is_some_and(|entry| rejected_drafts.contains(entry)))
        });
    }
    if !runtime_replies.is_empty() {
        deduplicated.retain(|item| {
            item.kind == OutputKind::Card
                || !(item.role == OutputRole::Assistant
                    && item
                        .provenance
                        .as_ref()
                        .and_then(|p| p.entry_id.as_ref())
                        .is_some_and(|entry| runtime_replies.contains(entry)))
        });
    }
    deduplicate_file_artifacts(&mut deduplicated);
    let first_card_index_by_turn: HashMap<String, usize> = deduplicated
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item.kind == OutputKind::Card
                && item
                    .provenance
                    .as_ref()
                    .and_then(|provenance| provenance.source.as_deref())
                    .is_some_and(|source| source.starts_with("emit_") && source.ends_with("_card"))
        })
        .fold(HashMap::new(), |mut indices, (index, item)| {
            indices.entry(item.turn_id.clone()).or_insert(index);
            indices
        });
    for (index, item) in deduplicated.iter_mut().enumerate() {
        if item.role != OutputRole::Assistant
            || !first_card_index_by_turn
                .get(&item.turn_id)
                .is_some_and(|first_card| index > *first_card)
        {
            continue;
        }
        let source_text = match &item.content {
            OutputContent::Document { document } => Some(document.source_markdown.clone()),
            OutputContent::Outcome { document, .. } => document
                .as_ref()
                .map(|document| document.source_markdown.clone()),
            _ => None,
        };
        let Some(followup) = source_text
            .as_deref()
            .and_then(vak_delivery::visible_card_followup)
        else {
            continue;
        };
        match &mut item.content {
            OutputContent::Document { document } => {
                document.metadata.insert("card_followup".into(), followup);
            }
            OutputContent::Outcome {
                document: Some(document),
                ..
            } => {
                document.metadata.insert("card_followup".into(), followup);
            }
            _ => {}
        }
    }
    timeline.items = deduplicated;
    // Each card's state as the person's actions left it (docs/design/96,
    // Q3), keyed by the Presentation entry the actions name.
    let mut states = vak_session::card_actions::card_states(session.chain_to_root());
    for item in &mut timeline.items {
        if let Some(id) = item
            .provenance
            .as_ref()
            .and_then(|provenance| provenance.presentation_id.as_ref())
            && let Some(state) = states.remove(id)
        {
            item.card_state = Some(state);
        }
    }
    apply_turn_evaluations(
        &mut timeline,
        &turn_evaluations,
        &turn_outcomes,
        &turn_evidence_state,
        &turn_human_review,
    );
    let placements: HashMap<_, _> = session
        .presentations()
        .into_iter()
        .map(|(id, record)| (id, record.placement.clone()))
        .collect();
    for item in &mut timeline.items {
        if let Some(placement) = item
            .provenance
            .as_ref()
            .and_then(|p| p.presentation_id.as_ref())
            .and_then(|id| placements.get(id))
        {
            item.placement = placement.clone();
        }
    }
    timeline.answers = answer_outlines(session, &timeline.items);
    timeline.cursor = chain_cursor(session);
    log_turns_with_no_visible_answer(session_id, &timeline);
    timeline
}

/// The outline of each answer whose model declared a plan
/// (docs/design/87-composed-answers.md §2): the latest plan the turn
/// recorded gives the sections, and each item is listed where the ledger
/// says it belongs. Nothing is attributed by position: a card that names no
/// planned deliverable is in no section.
fn answer_outlines(session: &SessionLog, items: &[OutputItem]) -> Vec<vak_delivery::AnswerOutline> {
    use std::collections::HashMap;
    // Per ledger turn: the plan it settled on. Per call: its ledger turn
    // and the deliverable it says it answers.
    let mut plans: HashMap<String, Vec<vak_intent::Deliverable>> = HashMap::new();
    let mut proposed: HashMap<String, (String, Vec<vak_intent::Deliverable>)> = HashMap::new();
    let mut calls: HashMap<String, (String, Option<String>)> = HashMap::new();
    let mut blocks_named: HashMap<(String, usize), Option<String>> = HashMap::new();
    let named = |input: &serde_json::Value| {
        input
            .get("deliverable")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
    };
    for entry in session.chain_to_root() {
        let (Some(turn), vak_session::EntryPayload::Message(record)) =
            (entry.at_turn.as_ref(), &entry.payload)
        else {
            continue;
        };
        for block in &record.message.content {
            match block {
                ContentBlock::ToolUse { id, name, input } => {
                    if name == "plan"
                        && let Ok(plan) = vak_tools::parse_plan_args(input)
                    {
                        proposed.insert(id.clone(), (turn.clone(), plan));
                    }
                    calls.insert(id.clone(), (turn.clone(), named(input)));
                    // An `answer` call's blocks each name their own.
                    if let Some(blocks) = input.get("blocks").and_then(serde_json::Value::as_array)
                    {
                        for (index, block) in blocks.iter().enumerate() {
                            blocks_named.insert((id.clone(), index), named(block));
                        }
                    }
                }
                ContentBlock::ToolResult {
                    tool_use_id,
                    is_error: false,
                    ..
                } => {
                    if let Some((turn, plan)) = proposed.remove(tool_use_id) {
                        plans.insert(turn, plan);
                    }
                }
                _ => {}
            }
        }
    }
    if plans.is_empty() && !items.iter().any(|item| item.placement.beside.is_some()) {
        return Vec::new();
    }
    // A card an `answer` block showed: the block it came from, by the
    // Presentation entry it was recorded as.
    let answer_blocks: HashMap<String, Option<String>> = session
        .presentations()
        .into_iter()
        .filter_map(|(entry_id, record)| match &record.source {
            vak_session::types::PresentationSource::Answer { tool_use_id, block } => blocks_named
                .get(&(tool_use_id.clone(), *block as usize))
                .map(|named| (entry_id, named.clone())),
            _ => None,
        })
        .collect();
    let call_of = |item: &OutputItem| {
        item.provenance
            .as_ref()
            .and_then(|provenance| provenance.tool_call_id.as_ref())
            .and_then(|call| calls.get(call))
    };
    let named_of = |item: &OutputItem| {
        item.provenance
            .as_ref()
            .and_then(|provenance| provenance.presentation_id.as_ref())
            .and_then(|id| answer_blocks.get(id))
            .cloned()
            .flatten()
            .or_else(|| call_of(item).and_then(|(_, answers)| answers.clone()))
    };
    let mut turns: Vec<&str> = Vec::new();
    for item in items {
        if !turns.contains(&item.turn_id.as_str()) {
            turns.push(&item.turn_id);
        }
    }
    let mut outlines = Vec::new();
    for turn_id in turns {
        let of_turn = || items.iter().filter(|item| item.turn_id == turn_id);
        // Resolve adjacency in the canonical visible source order, never after
        // summary/section reordering. Prose and result boundaries break pairs.
        let mut beside: Vec<[String; 2]> = Vec::new();
        let mut previous: Option<&OutputItem> = None;
        let mut paired = std::collections::HashSet::new();
        for item in of_turn() {
            if item.kind != OutputKind::Card {
                if item.role == OutputRole::Assistant
                    && matches!(item.content, OutputContent::Document { .. })
                {
                    previous = None;
                }
                continue;
            }
            if let Some(anchor) = previous
                && item.placement.beside.as_deref().is_some_and(|id| {
                    anchor
                        .provenance
                        .as_ref()
                        .and_then(|p| p.presentation_id.as_deref())
                        == Some(id)
                })
                && !paired.contains(&anchor.id)
                && named_of(anchor) == named_of(item)
            {
                beside.push([anchor.id.clone(), item.id.clone()]);
                paired.insert(anchor.id.clone());
                paired.insert(item.id.clone());
            }
            previous = Some(item);
        }
        let plan = of_turn().find_map(|item| call_of(item).and_then(|(turn, _)| plans.get(turn)));
        if plan.is_none() && beside.is_empty() {
            continue;
        }
        let mut sections: Vec<vak_delivery::AnswerSection> = plan
            .into_iter()
            .flatten()
            .map(|deliverable| vak_delivery::AnswerSection {
                name: deliverable.name.clone(),
                form: deliverable.form.as_str().to_string(),
                items: Vec::new(),
            })
            .collect();
        for item in of_turn().filter(|item| item.kind == OutputKind::Card) {
            let Some(named) = named_of(item) else {
                continue;
            };
            if let Some(section) = sections
                .iter_mut()
                .find(|section| section.name.trim().eq_ignore_ascii_case(&named))
            {
                // The model can satisfy a planned text deliverable with a
                // card. The outline follows the recorded delivery, not the
                // plan's initial format preference.
                section.form = "card".into();
                section.items.push(item.id.clone());
            }
        }
        // The reply's text is one block: it has a section only when the
        // plan has exactly one text deliverable.
        let mut texts = sections.iter_mut().filter(|section| section.form == "text");
        if let (Some(section), None) = (texts.next(), texts.next())
            && let Some(reply) = of_turn().rev().find(|item| {
                item.role == OutputRole::Assistant
                    && matches!(item.kind, OutputKind::Outcome | OutputKind::Message)
            })
        {
            section.items.push(reply.id.clone());
        }
        outlines.push(vak_delivery::AnswerOutline {
            turn_id: turn_id.to_string(),
            sections,
            beside,
        });
    }
    outlines
}

/// Mirrors the client's `Turn` filter (vak-client-ui's
/// `PresentationRenderer.tsx`): a turn is a *real answer* only if it carries
/// a `Document`/`Structured`/`Adaptive` item, or an `Outcome` with a
/// document attached. Everything else — bare outcomes, tool errors,
/// progress/retry/information — is invisible to a non-operator client.
///
/// If a turn has items but none of them qualify, the chat pane renders
/// nothing for it (now backstopped by a "no result" notice client-side,
/// but that's a fallback, not an explanation). Log it here so *why* is
/// inspectable from this process's log instead of only guessable from the
/// UI after the fact.
/// A `PresentationDocument` with no blocks and no source markdown renders
/// as a literal empty `<div>` client-side (see `PresentationDocumentView`
/// in vak-client-ui's PresentationRenderer.tsx) — the client now shows a
/// "no result" notice for that case too, but it still counts as "nothing"
/// for diagnostic purposes here.
fn document_has_content(document: &PresentationDocument) -> bool {
    !document.blocks.is_empty() || !document.source_markdown.trim().is_empty()
}

fn log_turns_with_no_visible_answer(session_id: &str, timeline: &OutputTimeline) {
    static WARNED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let mut turns: BTreeMap<&str, Vec<&OutputItem>> = BTreeMap::new();
    for item in &timeline.items {
        turns.entry(item.turn_id.as_str()).or_default().push(item);
    }
    for (turn_id, items) in turns {
        let has_real_answer = items.iter().any(|item| match &item.content {
            OutputContent::Document { document } => document_has_content(document),
            OutputContent::Structured { .. } | OutputContent::Adaptive { .. } => true,
            OutputContent::Outcome { document, .. } => {
                document.as_ref().is_some_and(document_has_content)
            }
            _ => false,
        });
        if has_real_answer {
            continue;
        }
        let non_progress: Vec<&&OutputItem> = items
            .iter()
            .filter(|item| {
                !matches!(
                    item.kind,
                    OutputKind::Progress | OutputKind::Retry | OutputKind::Information
                )
            })
            .collect();
        if non_progress.is_empty() {
            // Nothing happened in this turn yet (still streaming) — not a failure.
            continue;
        }
        let is_settled = non_progress
            .iter()
            .any(|item| !matches!(item.status, OutputStatus::Pending | OutputStatus::Running));
        if !is_settled {
            // The turn's only non-progress item is itself still Pending/Running
            // (e.g. a bare "Outcome/Running" marker) — this fires on every
            // poll of a turn that simply hasn't finished yet, not a failure.
            continue;
        }
        let kinds: Vec<String> = non_progress
            .iter()
            .map(|item| format!("{:?}/{:?}", item.kind, item.status))
            .collect();
        let fingerprint = format!("{session_id}\0{turn_id}\0{}", kinds.join(","));
        let mut warned = WARNED
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if warned.contains(&fingerprint) {
            continue;
        }
        // This is diagnostic process state, not session truth. Bound it so a
        // long-lived daemon cannot grow forever; clearing may repeat an old
        // warning once, which is preferable to unbounded memory or per-poll
        // log spam.
        if warned.len() >= 4096 {
            warned.clear();
        }
        warned.insert(fingerprint);
        tracing::warn!(
            session = %session_id,
            turn = %turn_id,
            count = non_progress.len(),
            "a turn produced no visible answer; the client shows a fallback notice"
        );
    }
}

fn chain_cursor(session: &SessionLog) -> Option<String> {
    session.chain_to_root().last().map(|entry| entry.id.clone())
}

/// The workspace files each call wrote, as the loop recorded them
/// (`CallEffect::FileWrite`), by call id: what a call's tool declared it
/// writes, never read from its name or argument keys.
fn written_files(chain: &[&vak_session::types::Entry]) -> HashMap<String, Vec<String>> {
    let mut written: HashMap<String, Vec<String>> = HashMap::new();
    for entry in chain {
        if let EntryPayload::CallEffect(record) = &entry.payload
            && let vak_session::types::CallEffect::FileWrite { path, .. } = &record.effect
        {
            let paths = written.entry(record.tool_use_id.clone()).or_default();
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
    }
    written
}

fn artifact_at(path: &str) -> ArtifactRef {
    let path = path.to_string();
    let filename = std::path::Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&path)
        .to_string();
    ArtifactRef {
        name: filename,
        media_type: media_type_for_path(&path),
        path: Some(path),
        description: None,
        size_bytes: None,
        status: Some(ArtifactStatus::InFolder),
    }
}

fn media_type_for_path(path: &str) -> Option<String> {
    let extension = std::path::Path::new(path).extension()?.to_str()?;
    Some(
        match extension.to_ascii_lowercase().as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            "html" | "htm" => "text/html",
            "css" => "text/css",
            "js" | "mjs" | "cjs" => "text/javascript",
            "ts" | "tsx" => "text/typescript",
            "pdf" => "application/pdf",
            "md" => "text/markdown",
            "json" => "application/json",
            "csv" => "text/csv",
            _ => "text/plain",
        }
        .into(),
    )
}

/// Whether an activity is something a person did or would recognise as
/// progress on their request. Everything else (admission, capacity,
/// diagnostics, retries, route fallbacks, voice and presentation accounting)
/// is runtime bookkeeping that stays in the ledger and never reaches a client.
fn is_user_facing_activity(activity: &vak_session::ActivityRecord) -> bool {
    match activity.kind {
        ActivityKind::Approval
        | ActivityKind::Worker
        | ActivityKind::CandidateComment
        | ActivityKind::CandidateRevision
        | ActivityKind::Ability => true,
        ActivityKind::Run => activity.label != "Request accepted",
        _ => false,
    }
}

fn activity_item(
    session_id: &str,
    entry_id: &str,
    timestamp: &str,
    turn_id: &str,
    ledger_turn: Option<&str>,
    activity: &vak_session::ActivityRecord,
) -> OutputItem {
    let status = match activity.status {
        ActivityStatus::Pending => OutputStatus::Pending,
        ActivityStatus::Running => OutputStatus::Running,
        ActivityStatus::Succeeded => OutputStatus::Succeeded,
        ActivityStatus::Failed => OutputStatus::Failed,
        ActivityStatus::Denied => OutputStatus::Denied,
        ActivityStatus::Cancelled => OutputStatus::Cancelled,
        ActivityStatus::Partial => OutputStatus::Partial,
    };
    let (role, kind, content) = match activity.kind {
        ActivityKind::Ability => (
            OutputRole::System,
            OutputKind::Ability,
            OutputContent::Ability {
                event: activity.data.get("event").cloned().unwrap_or_default(),
                skill: activity.data.get("skill").cloned().unwrap_or_default(),
                version: activity
                    .data
                    .get("version")
                    .and_then(|version| version.parse().ok()),
            },
        ),
        ActivityKind::Approval => (
            OutputRole::System,
            OutputKind::Approval,
            OutputContent::Approval {
                request_id: activity.data.get("request_id").cloned().unwrap_or_default(),
                tool: activity.data.get("tool").cloned().unwrap_or_default(),
                args_json: activity.data.get("args_json").cloned().unwrap_or_default(),
                reason: activity.detail.clone().unwrap_or_default(),
                expires_at: None,
            },
        ),
        ActivityKind::Retry => (
            OutputRole::System,
            OutputKind::Retry,
            OutputContent::Retry {
                attempt: activity
                    .data
                    .get("attempt")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1),
                delay_ms: activity
                    .data
                    .get("delay_ms")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0),
                reason: activity.detail.clone().unwrap_or_default(),
            },
        ),
        ActivityKind::Diagnostic if activity.label == "Outcome evaluation" => (
            OutputRole::System,
            OutputKind::Outcome,
            OutputContent::Outcome {
                summary: activity
                    .detail
                    .clone()
                    .unwrap_or_else(|| activity.label.clone()),
                document: None,
            },
        ),
        ActivityKind::Diagnostic
            if matches!(
                activity.label.as_str(),
                "Run paused" | "Run resumed" | "Run cancelled" | "Intervention queued"
            ) =>
        {
            (
                OutputRole::System,
                OutputKind::Information,
                OutputContent::Information {
                    label: activity.label.clone(),
                    detail: activity
                        .detail
                        .clone()
                        .or_else(|| activity.data.get("reason").cloned()),
                },
            )
        }
        ActivityKind::RouteFallback
        | ActivityKind::Diagnostic
        | ActivityKind::VoiceTranscript
        | ActivityKind::VoicePlayback
        | ActivityKind::PresentationSelection
        | ActivityKind::PresentationProposal
        | ActivityKind::PresentationFeedback
        | ActivityKind::CapacityProbe
        | ActivityKind::CapacityFeedback => (
            OutputRole::System,
            OutputKind::Information,
            OutputContent::Information {
                label: activity.label.clone(),
                detail: activity.detail.clone(),
            },
        ),
        ActivityKind::CandidateComment => (
            OutputRole::User,
            OutputKind::Information,
            OutputContent::Information {
                label: activity
                    .data
                    .get("path")
                    .map(|path| format!("Comment on {path}"))
                    .unwrap_or_else(|| "Comment on saved draft".into()),
                detail: activity.data.get("comment").cloned(),
            },
        ),
        ActivityKind::CandidateRevision => (
            OutputRole::Assistant,
            OutputKind::Progress,
            OutputContent::Progress {
                label: activity.label.clone(),
                detail: activity.detail.clone(),
                percent: None,
            },
        ),
        ActivityKind::Worker => (
            OutputRole::Worker,
            OutputKind::Progress,
            OutputContent::Progress {
                label: activity.label.clone(),
                detail: activity.detail.clone(),
                percent: None,
            },
        ),
        // Admission bookkeeping (`vak-server/src/lib.rs`'s `run`/`steering`
        // handlers append this to guard duplicate request ids). It is never
        // a real outcome; without this guard it fell into the generic
        // `ActivityKind::Run` arm below and rendered as a fake completed
        // "Request accepted" answer (docs/audits Finding 3).
        ActivityKind::Run if activity.label == "Request accepted" => (
            OutputRole::System,
            OutputKind::Information,
            OutputContent::Information {
                label: activity.label.clone(),
                detail: None,
            },
        ),
        ActivityKind::Run => (
            OutputRole::Assistant,
            if matches!(
                status,
                OutputStatus::Failed
                    | OutputStatus::Denied
                    | OutputStatus::Cancelled
                    | OutputStatus::Partial
            ) {
                OutputKind::Error
            } else {
                OutputKind::Outcome
            },
            if matches!(
                status,
                OutputStatus::Failed
                    | OutputStatus::Denied
                    | OutputStatus::Cancelled
                    | OutputStatus::Partial
            ) {
                // `activity.detail` is internal bookkeeping recorded at run
                // completion and can legitimately be a raw provider/internal
                // error string (docs/audits Finding 3); it never reaches an
                // item's text. The typed `status` alone selects one of the
                // same small human sentences `ClientEvent::RunFinished`
                // uses (`client_events::run_outcome_message`).
                use crate::client_events::{RunOutcome, run_outcome_message};
                let message = match status {
                    OutputStatus::Denied => "This step was denied.",
                    OutputStatus::Cancelled => run_outcome_message(RunOutcome::Stopped),
                    OutputStatus::Partial => run_outcome_message(RunOutcome::MaxTurns),
                    _ => run_outcome_message(RunOutcome::Failed),
                };
                OutputContent::Error {
                    message: message.into(),
                    source: Some("run".into()),
                    retryable: false,
                }
            } else {
                OutputContent::Outcome {
                    summary: activity
                        .detail
                        .clone()
                        .unwrap_or_else(|| activity.label.clone()),
                    document: None,
                }
            },
        ),
    };
    let actions = if activity.label == "Outcome evaluation" && status == OutputStatus::Succeeded {
        [
            ("accepted", "Accept result"),
            ("needs_work", "Mark needs work"),
            ("rejected", "Reject result"),
        ]
        .into_iter()
        .map(|(verdict, label)| DeliveryAction {
            id: format!("review-{verdict}-{}", activity.activity_id),
            label: label.into(),
            verb: "record_outcome_review".into(),
            data: {
                let mut data = BTreeMap::from([
                    ("session_id".into(), session_id.into()),
                    ("verdict".into(), verdict.into()),
                ]);
                if let Some(turn) = ledger_turn {
                    data.insert("turn".into(), turn.to_string());
                }
                data
            },
        })
        .collect()
    } else if kind == OutputKind::Approval && status == OutputStatus::Pending {
        let request_id = activity.data.get("request_id").cloned().unwrap_or_default();
        vec![
            DeliveryAction {
                id: format!("approve-{request_id}"),
                label: "Allow once".into(),
                verb: "resolve_approval".into(),
                data: BTreeMap::from([
                    ("request_id".into(), request_id.clone()),
                    ("verdict".into(), "allow".into()),
                ]),
            },
            DeliveryAction {
                id: format!("deny-{request_id}"),
                label: "Deny".into(),
                verb: "resolve_approval".into(),
                data: BTreeMap::from([
                    ("request_id".into(), request_id),
                    ("verdict".into(), "deny".into()),
                ]),
            },
        ]
    } else {
        Vec::new()
    };
    OutputItem {
        placement: Default::default(),
        card_state: None,
        id: activity.activity_id.clone(),
        timestamp: timestamp.into(),
        turn_id: turn_id.into(),
        role,
        kind,
        status,
        outcome: None,
        content,
        provenance: Some(OutputProvenance {
            session_id: Some(session_id.into()),
            entry_id: Some(entry_id.into()),
            tool_call_id: None,
            source: Some("activity_ledger".into()),
            presentation_id: None,
        }),
        actions,
        fallback_text: activity
            .detail
            .clone()
            .unwrap_or_else(|| activity.label.clone()),
    }
}

pub(crate) fn live_event(session_id: &str, event: AgentEvent) -> Option<OutputStreamEvent> {
    let now = chrono::Utc::now().to_rfc3339();
    match event {
        // A card as recorded, drawn while the run goes on; the settled
        // snapshot replaces it (docs/design/96, live turn).
        AgentEvent::CardRecorded {
            tool_use_id,
            presentation_id,
            record,
        } => {
            let output = vak_delivery::StructuredOutput {
                semantic_type: record.semantic_type.clone(),
                block: record.block.clone().unwrap_or_else(|| {
                    if vak_delivery::is_form(&record.semantic_type) {
                        record.semantic_type.clone()
                    } else {
                        String::new()
                    }
                }),
                schema_version: u16::try_from(record.schema_version).unwrap_or(u16::MAX),
                skill_id: record.skill_id.clone(),
                skill_version: record.skill_version.clone(),
                payload: record.payload.clone(),
            };
            Some(OutputStreamEvent::ItemStarted {
                item: OutputItem {
                    placement: record.placement.clone(),
                    card_state: None,
                    id: format!("{tool_use_id}-structured-0"),
                    timestamp: now,
                    turn_id: record.turn_id.clone(),
                    role: OutputRole::Tool,
                    kind: OutputKind::Card,
                    status: OutputStatus::Succeeded,
                    outcome: None,
                    fallback_text: structured_markdown(&output),
                    content: OutputContent::Structured { output },
                    provenance: Some(OutputProvenance {
                        session_id: Some(session_id.into()),
                        entry_id: None,
                        tool_call_id: Some(tool_use_id),
                        source: Some("live_event".into()),
                        presentation_id: Some(presentation_id),
                    }),
                    actions: Vec::new(),
                },
            })
        }
        AgentEvent::TurnStart { turn } => Some(OutputStreamEvent::ItemStarted {
            item: OutputItem {
                placement: Default::default(),
                card_state: None,
                id: "live-assistant".into(),
                timestamp: now,
                turn_id: format!("turn-{turn}"),
                role: OutputRole::Assistant,
                kind: OutputKind::Outcome,
                status: OutputStatus::Running,
                outcome: None,
                content: OutputContent::Document {
                    document: compile_markdown(""),
                },
                provenance: Some(OutputProvenance {
                    session_id: Some(session_id.into()),
                    entry_id: None,
                    tool_call_id: None,
                    source: Some("live_event".into()),
                    presentation_id: None,
                }),
                actions: Vec::new(),
                fallback_text: String::new(),
            },
        }),
        AgentEvent::Stream(vak_llm::StreamEvent::TextDelta { delta, partial }) => {
            Some(OutputStreamEvent::TextDelta {
                item_id: "live-assistant".into(),
                delta,
                text: partial.text_content(),
            })
        }
        AgentEvent::ToolCallStart { id, name, .. } => Some(OutputStreamEvent::ItemStarted {
            item: live_item(
                session_id,
                id,
                now,
                OutputRole::Tool,
                OutputKind::Progress,
                OutputStatus::Running,
                OutputContent::Progress {
                    label: name.clone(),
                    detail: None,
                    percent: None,
                },
                format!("{name} running"),
            ),
        }),
        AgentEvent::ToolCallEnd {
            id,
            name,
            is_error,
            result_preview,
        } => Some(OutputStreamEvent::ItemReplaced {
            item: live_item(
                session_id,
                id,
                now,
                OutputRole::Tool,
                if is_error {
                    OutputKind::Error
                } else {
                    OutputKind::Progress
                },
                if is_error {
                    OutputStatus::Failed
                } else {
                    OutputStatus::Succeeded
                },
                if is_error {
                    OutputContent::Error {
                        message: result_preview
                            .clone()
                            .unwrap_or_else(|| "Tool failed".into()),
                        source: Some(name),
                        retryable: false,
                    }
                } else {
                    OutputContent::Progress {
                        label: name,
                        detail: result_preview.clone(),
                        percent: None,
                    }
                },
                result_preview.unwrap_or_default(),
            ),
        }),
        AgentEvent::Sandbox(vak_tools::SandboxEvent::ArtifactGenerated {
            execution_id,
            path,
            mime_type,
            size_bytes,
        }) => Some(OutputStreamEvent::ItemStarted {
            item: OutputItem {
                placement: Default::default(),
                card_state: None,
                id: format!("artifact-{execution_id}-{path}"),
                timestamp: now,
                turn_id: format!("sandbox-{execution_id}"),
                role: OutputRole::Tool,
                kind: OutputKind::Artifact,
                status: OutputStatus::Succeeded,
                outcome: None,
                content: OutputContent::Artifact {
                    artifact: ArtifactRef {
                        name: std::path::Path::new(&path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(&path)
                            .to_string(),
                        path: Some(path.clone()),
                        media_type: Some(mime_type.clone()),
                        description: None,
                        size_bytes: Some(size_bytes),
                        status: None,
                    },
                },
                provenance: Some(OutputProvenance {
                    session_id: Some(session_id.into()),
                    entry_id: None,
                    tool_call_id: Some(execution_id.clone()),
                    source: Some("sandbox_artifact".into()),
                    presentation_id: None,
                }),
                // A review action is added after settlement only when an
                // ExecutionStarted receipt proves a reviewable scratch root.
                actions: sandbox_artifact_actions(&execution_id, &path, false),
                fallback_text: format!("Generated artifact: {path}"),
            },
        }),
        AgentEvent::RetryScheduled {
            attempt,
            delay_ms,
            reason,
        } => Some(OutputStreamEvent::ItemStarted {
            item: live_item(
                session_id,
                format!("retry-{attempt}"),
                now,
                OutputRole::System,
                OutputKind::Retry,
                OutputStatus::Running,
                OutputContent::Retry {
                    attempt,
                    delay_ms,
                    reason: reason.clone(),
                },
                reason,
            ),
        }),
        AgentEvent::RouteFallback {
            to_provider,
            to_model,
        } => Some(OutputStreamEvent::ItemStarted {
            item: live_item(
                session_id,
                format!("route-{to_provider}-{to_model}"),
                now,
                OutputRole::System,
                OutputKind::Information,
                OutputStatus::Running,
                OutputContent::Information {
                    label: "Route fallback".into(),
                    detail: Some(format!("{to_provider} · {to_model}")),
                },
                format!("Continuing with {to_provider} · {to_model}"),
            ),
        }),
        AgentEvent::WorkerStarted { label } => Some(OutputStreamEvent::ItemStarted {
            item: live_item(
                session_id,
                format!("worker-{label}"),
                now,
                OutputRole::Worker,
                OutputKind::Progress,
                OutputStatus::Running,
                OutputContent::Progress {
                    label: label.clone(),
                    detail: Some("Worker started".into()),
                    percent: None,
                },
                format!("{label} started"),
            ),
        }),
        AgentEvent::WorkerFinished {
            label,
            is_error,
            elapsed_ms,
        } => Some(OutputStreamEvent::ItemReplaced {
            item: live_item(
                session_id,
                format!("worker-{label}"),
                now,
                OutputRole::Worker,
                if is_error {
                    OutputKind::Error
                } else {
                    OutputKind::Progress
                },
                if is_error {
                    OutputStatus::Failed
                } else {
                    OutputStatus::Succeeded
                },
                if is_error {
                    OutputContent::Error {
                        message: format!("{label} failed after {elapsed_ms} ms"),
                        source: Some("worker".into()),
                        retryable: false,
                    }
                } else {
                    OutputContent::Progress {
                        label: label.clone(),
                        detail: Some(format!("Completed in {elapsed_ms} ms")),
                        percent: Some(100),
                    }
                },
                format!("{label} finished in {elapsed_ms} ms"),
            ),
        }),
        // The managed-work projection is not rendered from the presentation
        // timeline (the work panel reads `GET /sessions/:id/work` instead);
        // it previously leaked its full `serde_json::to_string` dump into an
        // `Information` item's `detail`/`fallback_text` (docs/audits Finding
        // 3 -- a raw JSON blob is exactly what a client must never receive).
        AgentEvent::WorkState { .. } => None,
        AgentEvent::ApprovalRequested {
            id,
            tool,
            args_json,
            reason,
        } => Some(OutputStreamEvent::ItemStarted {
            item: OutputItem {
                placement: Default::default(),
                card_state: None,
                id: id.clone(),
                timestamp: now,
                turn_id: "live".into(),
                role: OutputRole::System,
                kind: OutputKind::Approval,
                status: OutputStatus::Pending,
                outcome: None,
                content: OutputContent::Approval {
                    request_id: id.clone(),
                    tool,
                    args_json,
                    reason: reason.clone(),
                    expires_at: None,
                },
                provenance: Some(OutputProvenance {
                    session_id: Some(session_id.into()),
                    entry_id: None,
                    tool_call_id: None,
                    source: Some("live_event".into()),
                    presentation_id: None,
                }),
                actions: vec![
                    DeliveryAction {
                        id: format!("approve-{id}"),
                        label: "Allow once".into(),
                        verb: "resolve_approval".into(),
                        data: BTreeMap::from([
                            ("request_id".into(), id.clone()),
                            ("verdict".into(), "allow".into()),
                        ]),
                    },
                    DeliveryAction {
                        id: format!("deny-{id}"),
                        label: "Deny".into(),
                        verb: "resolve_approval".into(),
                        data: BTreeMap::from([
                            ("request_id".into(), id.clone()),
                            ("verdict".into(), "deny".into()),
                        ]),
                    },
                ],
                fallback_text: reason,
            },
        }),
        AgentEvent::RunFinished { outcome, .. } => Some(OutputStreamEvent::ItemCompleted {
            item_id: "live-assistant".into(),
            status: match outcome {
                vak_agent::AgentRunOutcome::Completed => OutputStatus::Succeeded,
                vak_agent::AgentRunOutcome::Stopped => OutputStatus::Cancelled,
                vak_agent::AgentRunOutcome::MaxTurns => OutputStatus::Partial,
                vak_agent::AgentRunOutcome::Failed => OutputStatus::Failed,
            },
        }),
        _ => None,
    }
}

pub(crate) fn project_frame(
    timeline: &mut OutputTimeline,
    framed: crate::events::SeqEvent,
) -> Option<vak_delivery::OutputStreamFrame> {
    let sequence = framed.seq;
    if timeline
        .covered_sequence
        .is_some_and(|covered| sequence <= covered)
    {
        return None;
    }
    let mut event = live_event(&timeline.session_id, framed.event)?;
    let active = timeline
        .items
        .iter()
        .rev()
        .find(|item| item.id.starts_with("live-assistant-"))
        .map(|item| (item.id.clone(), item.turn_id.clone()));
    match &mut event {
        OutputStreamEvent::ItemStarted { item } | OutputStreamEvent::ItemReplaced { item } => {
            if item.id == "live-assistant" {
                item.id = format!("live-assistant-{sequence}");
                item.turn_id = format!("live-turn-{sequence}");
            } else if item.turn_id == "live"
                && let Some((_, turn)) = &active
            {
                item.turn_id = turn.clone();
            }
        }
        OutputStreamEvent::TextDelta { item_id, .. }
        | OutputStreamEvent::ItemCompleted { item_id, .. } => {
            if let Some((id, _)) = &active {
                *item_id = id.clone();
            }
        }
        OutputStreamEvent::Snapshot { .. } => {}
    }
    apply_stream_event(timeline, event.clone());
    timeline.cursor = Some(format!("live:{sequence}"));
    // The runtime consumed the accumulated snapshot above. Transport only
    // the delta; resync/settlement supplies an authoritative full snapshot.
    if let OutputStreamEvent::TextDelta { text, .. } = &mut event {
        text.clear();
    }
    // No `timeline.clone()` here: a live frame carries only its own small
    // delta. The per-handle projector in `register_handle` applies every
    // event through this function whether or not a client is subscribed, so
    // a snapshot built here would be an unconditional full-timeline clone
    // per event regardless of demand (docs/audits Finding 2).
    Some(vak_delivery::OutputStreamFrame {
        sequence: Some(sequence),
        delta: Some(event),
        snapshot: None,
    })
}

/// Rebase a live presentation stream onto the durable projection at a run
/// boundary. `snapshot` alone is authoritative here — `delta` stays `None`
/// rather than carrying the same timeline a second time as
/// `OutputStreamEvent::Snapshot` (docs/audits Finding 2: a settlement frame
/// previously measured up to 9.3 MB by sending it twice).
pub(crate) fn settled_frame(
    sequence: u64,
    timeline: &OutputTimeline,
) -> vak_delivery::OutputStreamFrame {
    let mut snapshot = timeline.clone();
    // The handle's background projector receives the same RunFinished event
    // and may win the mutex race, changing only the cursor back to `live:*`.
    // A settlement frame is an authoritative replacement regardless of that
    // scheduling order, so give it an explicitly non-live cursor.
    if snapshot
        .cursor
        .as_deref()
        .is_some_and(|cursor| cursor.starts_with("live:"))
    {
        snapshot.cursor = Some(format!("settled:{sequence}"));
    }
    vak_delivery::OutputStreamFrame {
        sequence: Some(sequence),
        delta: None,
        snapshot: Some(snapshot),
    }
}

pub(crate) fn apply_stream_event(timeline: &mut OutputTimeline, event: OutputStreamEvent) {
    match event {
        OutputStreamEvent::Snapshot { timeline: snapshot } => *timeline = snapshot,
        OutputStreamEvent::ItemStarted { item } | OutputStreamEvent::ItemReplaced { item } => {
            // The background event consumer may trail the settlement snapshot.
            // A late live card cannot overwrite its authoritative durable item.
            if item.provenance.as_ref().and_then(|p| p.source.as_deref()) == Some("live_event")
                && timeline.items.iter().any(|existing| {
                    existing.id == item.id
                        && existing
                            .provenance
                            .as_ref()
                            .and_then(|p| p.source.as_deref())
                            != Some("live_event")
                })
            {
                return;
            }
            if let Some(replaced) = &item.placement.replaces {
                timeline.items.retain(|candidate| {
                    candidate
                        .provenance
                        .as_ref()
                        .and_then(|p| p.presentation_id.as_ref())
                        != Some(replaced)
                });
            }
            if let Some(existing) = timeline
                .items
                .iter_mut()
                .find(|candidate| candidate.id == item.id)
            {
                *existing = item;
            } else {
                timeline.items.push(item);
            }
        }
        OutputStreamEvent::TextDelta {
            item_id,
            delta,
            text,
        } => {
            if let Some(item) = timeline
                .items
                .iter_mut()
                .find(|candidate| candidate.id == item_id)
            {
                let text = if text.is_empty() {
                    format!("{}{delta}", item.fallback_text)
                } else {
                    text
                };
                item.fallback_text = text.clone();
                // The renderer already supports source-only live documents.
                // Compile once on completion/durable settlement, rather than
                // parsing every growing prefix on the stream's hot path.
                item.content = OutputContent::Document {
                    document: PresentationDocument {
                        source_markdown: text,
                        ..Default::default()
                    },
                };
            }
        }
        OutputStreamEvent::ItemCompleted { item_id, status } => {
            if let Some(item) = timeline
                .items
                .iter_mut()
                .find(|candidate| candidate.id == item_id)
            {
                item.status = status;
                if matches!(item.content, OutputContent::Document { .. }) {
                    item.content = OutputContent::Document {
                        document: compile_markdown(item.fallback_text.clone()),
                    };
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn live_item(
    session_id: &str,
    id: String,
    timestamp: String,
    role: OutputRole,
    kind: OutputKind,
    status: OutputStatus,
    content: OutputContent,
    fallback_text: String,
) -> OutputItem {
    OutputItem {
        placement: Default::default(),
        card_state: None,
        id,
        timestamp,
        turn_id: "live".into(),
        role,
        kind,
        status,
        outcome: None,
        content,
        provenance: Some(OutputProvenance {
            session_id: Some(session_id.into()),
            entry_id: None,
            tool_call_id: None,
            source: Some("live_event".into()),
            presentation_id: None,
        }),
        actions: Vec::new(),
        fallback_text,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::activity_item;
    use super::snapshot;
    use std::collections::{BTreeMap, HashMap};
    use std::path::PathBuf;
    use vak_delivery::{
        ArtifactStatus, OutputContent, OutputItem, OutputKind, OutputProvenance, OutputRole,
        OutputStatus, OutputTimeline, ResultOutcome,
    };
    use vak_llm::{ContentBlock, Message, Role};
    use vak_session::{
        ActivityKind, ActivityRecord, ActivityStatus, FrozenContract, MessageRecord, SessionHeader,
        SessionLog,
    };

    #[test]
    fn settled_frame_rebases_snapshot_to_durable_history_and_sends_no_delta() {
        let mut durable = OutputTimeline::empty("session-1");
        durable.cursor = Some("live:41".into());
        let frame = super::settled_frame(42, &durable);
        assert_eq!(frame.sequence, Some(42));
        let snapshot = frame
            .snapshot
            .expect("settlement always carries a snapshot");
        assert_eq!(snapshot.cursor.as_deref(), Some("settled:42"));
        // A settlement frame carries the timeline exactly once: `snapshot`
        // is the sole authority and `delta` stays `None` rather than
        // repeating it as `OutputStreamEvent::Snapshot` (docs/audits Finding
        // 2 -- doubling this previously measured up to 9.3 MB per answer).
        assert!(frame.delta.is_none());
    }

    #[test]
    fn project_frame_carries_only_a_delta_never_a_snapshot() {
        let mut timeline = OutputTimeline::empty("session-1");
        let framed = crate::events::SeqEvent {
            seq: 7,
            event: vak_agent::AgentEvent::TurnStart { turn: 0 },
        };
        let frame = super::project_frame(&mut timeline, framed).expect("TurnStart projects");
        assert_eq!(frame.sequence, Some(7));
        assert!(frame.delta.is_some());
        assert!(frame.snapshot.is_none());
    }

    #[test]
    fn streamed_text_frames_send_deltas_and_preserve_partial_snapshots() {
        let mut timeline = OutputTimeline::empty("stream-contract");
        super::project_frame(
            &mut timeline,
            crate::events::SeqEvent {
                seq: 1,
                event: vak_agent::AgentEvent::TurnStart { turn: 0 },
            },
        );
        for (index, (delta, text)) in [("Hello ", "Hello "), ("world", "Hello world")]
            .into_iter()
            .enumerate()
        {
            let frame = super::project_frame(
                &mut timeline,
                crate::events::SeqEvent {
                    seq: index as u64 + 2,
                    event: vak_agent::AgentEvent::Stream(vak_llm::StreamEvent::TextDelta {
                        delta: delta.into(),
                        partial: vak_llm::types::AssistantMessage {
                            content: vec![ContentBlock::text(text)],
                            ..vak_llm::types::AssistantMessage::empty("projection-fixture")
                        },
                    }),
                },
            )
            .expect("text frame");
            let wire = serde_json::to_value(&frame).expect("wire");
            assert!(
                wire["delta"].get("text").is_none(),
                "no growing prefix on wire"
            );
            assert_eq!(wire["delta"]["delta"], delta);
            let item = &timeline.items[0];
            assert_eq!(item.fallback_text, text);
            let OutputContent::Document { document } = &item.content else {
                panic!("document")
            };
            assert_eq!(document.source_markdown, text);
            assert!(document.blocks.is_empty(), "no prefix markdown compilation");
        }
        let resync = super::settled_frame(4, &timeline);
        assert_eq!(
            resync.snapshot.expect("resync").items[0].fallback_text,
            "Hello world"
        );
    }

    #[allow(clippy::disallowed_macros)] // prints a measurement for the person running it
    /// Local deterministic workload; prints measurements only when explicitly
    /// requested. The baseline reconstructs the previous prefix compile/wire
    /// behavior, while the changed path uses the real frame projector.
    #[test]
    #[ignore = "local performance measurement"]
    fn measure_streaming_projection_prefix_work() {
        const FRAMES: usize = 1000;
        let chunk = "A measured **answer** with detail. ";
        let mut text = String::new();
        let mut baseline_bytes = 0;
        let started = std::time::Instant::now();
        for _ in 0..FRAMES {
            text.push_str(chunk);
            std::hint::black_box(vak_delivery::compile_markdown(text.clone()));
            baseline_bytes += serde_json::to_vec(&serde_json::json!({"sequence": 1, "delta": {
                "type": "text_delta", "item_id": "live-assistant-1", "delta": chunk, "text": text
            }, "snapshot": null}))
            .expect("baseline wire")
            .len();
        }
        let baseline_time = started.elapsed();
        let mut timeline = OutputTimeline::empty("stream-measurement");
        super::project_frame(
            &mut timeline,
            crate::events::SeqEvent {
                seq: 1,
                event: vak_agent::AgentEvent::TurnStart { turn: 0 },
            },
        );
        text.clear();
        let mut wire_bytes = 0;
        let started = std::time::Instant::now();
        for index in 0..FRAMES {
            text.push_str(chunk);
            let frame = super::project_frame(
                &mut timeline,
                crate::events::SeqEvent {
                    seq: index as u64 + 2,
                    event: vak_agent::AgentEvent::Stream(vak_llm::StreamEvent::TextDelta {
                        delta: chunk.into(),
                        partial: vak_llm::types::AssistantMessage {
                            content: vec![ContentBlock::text(text.clone())],
                            ..vak_llm::types::AssistantMessage::empty("projection-fixture")
                        },
                    }),
                },
            )
            .expect("frame");
            wire_bytes += serde_json::to_vec(&frame).expect("wire").len();
        }
        eprintln!(
            "frames={FRAMES} source_bytes={} reconstructed_prefix_baseline_bytes={baseline_bytes} delta_wire_bytes={wire_bytes} baseline_us={} changed_us={}",
            text.len(),
            baseline_time.as_micros(),
            started.elapsed().as_micros()
        );
        assert_eq!(timeline.items[0].fallback_text, text);
        assert!(wire_bytes < baseline_bytes / 10);
    }

    /// The handle's live timeline is what `/presentation` serves while a run
    /// holds the ledger, so a page reloaded mid-turn draws the cards already
    /// recorded (docs/design/96, live turn).
    #[test]
    fn a_running_turns_timeline_carries_its_recorded_cards() {
        let mut timeline = OutputTimeline::empty("session-1");
        super::project_frame(
            &mut timeline,
            crate::events::SeqEvent {
                seq: 3,
                event: vak_agent::AgentEvent::TurnStart { turn: 0 },
            },
        );
        let payload = serde_json::json!({
            "title": "Boil an egg",
            "steps": [{"label": "Boil", "text": "Boil water"}],
        });
        let record = vak_session::types::PresentationRecord {
            placement: Default::default(),
            turn_id: "directive-1".into(),
            source: vak_session::types::PresentationSource::ToolCall {
                tool_use_id: "call-1".into(),
            },
            semantic_type: "steps".into(),
            skill_id: "steps".into(),
            skill_version: "1".into(),
            schema_version: 2,
            payload_digest: vak_session::types::payload_digest(&payload),
            payload,
            derived_from: Vec::new(),
            title: "Boil an egg".into(),
            identity_digest: String::new(),
            block: Some("steps".into()),
            data_call_id: None,
        };
        super::project_frame(
            &mut timeline,
            crate::events::SeqEvent {
                seq: 4,
                event: vak_agent::AgentEvent::CardRecorded {
                    tool_use_id: "call-1".into(),
                    presentation_id: "pres-1".into(),
                    record: Box::new(record),
                },
            },
        );
        let card = timeline
            .items
            .iter()
            .find(|item| item.id == "call-1-structured-0")
            .expect("the recorded card is in the live timeline");
        assert_eq!(card.kind, OutputKind::Card);
        assert_eq!(card.turn_id, "directive-1");
        let provenance = card.provenance.as_ref().expect("provenance");
        assert_eq!(provenance.source.as_deref(), Some("live_event"));
        assert_eq!(provenance.presentation_id.as_deref(), Some("pres-1"));
    }

    /// Writes the `Presentation` entry a real turn would have written at
    /// card validation (docs/design/68-context-engine.md §10), so these
    /// fixture ledgers exercise the same projection path production does:
    /// reading the entry, never rebuilding the card from `tool_use.input`.
    fn append_presentation_for_call(
        log: &mut SessionLog,
        tool: &str,
        tool_use_id: &str,
        args: &serde_json::Value,
    ) {
        append_presentation(
            log,
            tool,
            vak_session::types::PresentationSource::ToolCall {
                tool_use_id: tool_use_id.into(),
            },
            args,
        );
    }

    fn append_presentation(
        log: &mut SessionLog,
        tool: &str,
        source: vak_session::types::PresentationSource,
        args: &serde_json::Value,
    ) {
        let skills = vak_delivery::built_in_skill_registry();
        // Placement fields were the model's to write before 8.28.0; history
        // keeps them on the record, never in the card.
        let mut card_args = args.clone();
        if let Some(fields) = card_args.as_object_mut() {
            for key in ["deliverable", "beside_previous", "replaces", "then"] {
                fields.remove(key);
            }
        }
        let info = vak_core::presentation_tools::presentation_info(tool, &card_args, &skills)
            .unwrap_or_else(|| panic!("fixture call must validate: {tool} {args:.600}"));
        let turn_id = log.latest_directive_entry_id().unwrap_or_default();
        let payload_digest = vak_session::types::payload_digest(&info.payload);
        let placement = log.card_placement(
            &turn_id,
            args.get("deliverable")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            args.get("beside_previous") == Some(&serde_json::Value::Bool(true)),
            args.get("replaces")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
        );
        log.append_presentation(vak_session::types::PresentationRecord {
            placement,
            block: None,
            data_call_id: None,
            turn_id,
            source,
            semantic_type: info.semantic_type,
            skill_id: info.skill_id,
            skill_version: info.skill_version,
            schema_version: info.schema_version,
            payload: info.payload,
            payload_digest,
            derived_from: Vec::new(),
            title: info.title,
            identity_digest: info.identity_digest,
        })
        .expect("append presentation");
    }

    #[test]
    fn runtime_bookkeeping_never_reaches_the_client_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = SessionLog::create(
            dir.path().join("bookkeeping.jsonl"),
            SessionHeader {
                space: None,
                run: None,
                cause: None,
                agent: None,
                session_id: "bookkeeping".into(),
                created_at: chrono::Utc::now(),
                cwd: PathBuf::from("/tmp/project"),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
                conversation: None,
                contract: FrozenContract {
                    app_version: "test".into(),
                    provider: "test".into(),
                    model: "test".into(),
                    route_ladder: Vec::new(),
                    route_objective: String::new(),
                    route_annotations: Vec::new(),
                    system_prompt: String::new(),
                    permission_mode: "read-only".into(),
                    capabilities: Vec::new(),
                    prompt_layers: Vec::new(),
                },
            },
        )
        .expect("create session");
        let activity = |id: &str, kind: ActivityKind, label: &str| ActivityRecord {
            activity_id: id.into(),
            kind,
            status: ActivityStatus::Succeeded,
            label: label.into(),
            detail: None,
            data: BTreeMap::new(),
        };
        for record in [
            activity("admission-r1", ActivityKind::Run, "Request accepted"),
            activity(
                "probe-1",
                ActivityKind::CapacityProbe,
                "Capacity profile bound",
            ),
            activity(
                "feedback-1",
                ActivityKind::CapacityFeedback,
                "Capacity profile updated",
            ),
            activity("diag-1", ActivityKind::Diagnostic, "prefix-changed"),
            activity("retry-1", ActivityKind::Retry, "Retry attempt 1"),
            activity("route-1", ActivityKind::RouteFallback, "Route fallback"),
        ] {
            log.append_activity(record).expect("activity");
        }
        log.append_goal_update(vak_intent::GoalUpdate {
            revision: 1,
            relation: vak_intent::GoalRelation::New,
            request: "say hi".into(),
            supersedes_revision: None,
            explicit: false,
        })
        .expect("goal update");
        log.append_message(MessageRecord {
            message: Message::user_text("say hi"),
            meta: None,
        })
        .expect("directive");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text("Hi.")]),
            meta: None,
        })
        .expect("answer");

        let timeline = snapshot("bookkeeping", &log);
        let leaked: Vec<&str> = timeline
            .items
            .iter()
            .filter(|item| item.role == OutputRole::System)
            .map(|item| item.fallback_text.as_str())
            .collect();
        assert!(
            leaked.is_empty(),
            "internal items reached the client: {leaked:?}"
        );
        assert!(
            timeline
                .items
                .iter()
                .any(|item| item.fallback_text.contains("Hi.")),
            "the answer itself must still be projected"
        );
    }

    #[test]
    fn a_workers_cards_project_as_cards_of_the_task_call() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = SessionLog::create(
            dir.path().join("delegated.jsonl"),
            SessionHeader {
                space: None,
                run: None,
                cause: None,
                agent: None,
                session_id: "delegated".into(),
                created_at: chrono::Utc::now(),
                cwd: PathBuf::from("/tmp/project"),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
                conversation: None,
                contract: FrozenContract {
                    app_version: "test".into(),
                    provider: "test".into(),
                    model: "test".into(),
                    route_ladder: Vec::new(),
                    route_objective: String::new(),
                    route_annotations: Vec::new(),
                    system_prompt: String::new(),
                    permission_mode: "read-only".into(),
                    capabilities: Vec::new(),
                    prompt_layers: Vec::new(),
                },
            },
        )
        .expect("create session");
        log.append_message(MessageRecord {
            message: Message::user_text("chart it and summarise it"),
            meta: None,
        })
        .expect("directive");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::ToolUse {
                id: "task-1".into(),
                name: "task".into(),
                input: serde_json::json!({"prompt": "chart it"}),
            }]),
            meta: None,
        })
        .expect("task call");
        let skills = vak_delivery::built_in_skill_registry();
        let turn_id = log.latest_directive_entry_id().unwrap_or_default();
        let mut ids = Vec::new();
        for summary in ["revenue", "cost"] {
            let info = vak_core::presentation_tools::presentation_info(
                "emit_chart_card",
                &serde_json::json!({"semantic_type":"chart","chart_type":"line","data":[{"series":"s","x":"Mon","y":1}],"accessible_summary":summary}),
                &skills,
            )
            .expect("fixture card validates");
            let entry = log
                .append_presentation(vak_session::types::PresentationRecord {
                    placement: Default::default(),
                    block: None,
                    data_call_id: None,
                    turn_id: turn_id.clone(),
                    source: vak_session::types::PresentationSource::Delegated {
                        tool_use_id: "task-1".into(),
                        worker_session_id: "child-1".into(),
                    },
                    semantic_type: info.semantic_type,
                    skill_id: info.skill_id,
                    skill_version: info.skill_version,
                    schema_version: info.schema_version,
                    payload_digest: vak_session::types::payload_digest(&info.payload),
                    payload: info.payload,
                    derived_from: vec!["task-1".into()],
                    title: info.title,
                    identity_digest: info.identity_digest,
                })
                .expect("delegated presentation");
            ids.push(entry.id);
        }
        log.append_message(MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "task-1".into(),
                    content: "Revenue rose and cost held.".into(),
                    is_error: false,
                }],
            },
            meta: None,
        })
        .expect("task result");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text("Both charts are above.")]),
            meta: None,
        })
        .expect("answer");

        let timeline = snapshot("delegated", &log);
        let cards: Vec<&OutputItem> = timeline
            .items
            .iter()
            .filter(|item| item.kind == OutputKind::Card)
            .collect();
        assert_eq!(cards.len(), 2, "both of the worker's cards are shown");
        for (card, id) in cards.iter().zip(&ids) {
            let provenance = card.provenance.as_ref().expect("provenance");
            assert_eq!(provenance.tool_call_id.as_deref(), Some("task-1"));
            assert_eq!(provenance.presentation_id.as_deref(), Some(id.as_str()));
        }
    }

    /// A call's artifact is the file the loop recorded it writing, whatever
    /// the tool is named; a read records no write and claims nothing.
    #[test]
    fn a_recorded_write_projects_an_artifact_and_a_read_does_not() {
        use vak_session::types::{CallEffect, CallEffectRecord, Entry, EntryPayload};
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = SessionLog::create(
            dir.path().join("written.jsonl"),
            SessionHeader {
                space: None,
                run: None,
                cause: None,
                agent: None,
                session_id: "written".into(),
                created_at: chrono::Utc::now(),
                cwd: PathBuf::from("/tmp/project"),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
                conversation: None,
                contract: FrozenContract {
                    app_version: "test".into(),
                    provider: "test".into(),
                    model: "test".into(),
                    route_ladder: Vec::new(),
                    route_objective: String::new(),
                    route_annotations: Vec::new(),
                    system_prompt: String::new(),
                    permission_mode: "read-only".into(),
                    capabilities: Vec::new(),
                    prompt_layers: Vec::new(),
                },
            },
        )
        .expect("create session");
        log.append_message(MessageRecord {
            message: Message::user_text("save a note"),
            meta: None,
        })
        .expect("directive");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![
                ContentBlock::ToolUse {
                    id: "w1".into(),
                    name: "keep_note".into(),
                    input: serde_json::json!({ "where": "report.md" }),
                },
                ContentBlock::ToolUse {
                    id: "r1".into(),
                    name: "look".into(),
                    input: serde_json::json!({ "path": "a.md" }),
                },
            ]),
            meta: None,
        })
        .expect("calls");
        log.append_message(MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![
                    ContentBlock::ToolResult {
                        tool_use_id: "w1".into(),
                        content: "saved".into(),
                        is_error: false,
                    },
                    ContentBlock::ToolResult {
                        tool_use_id: "r1".into(),
                        content: "text".into(),
                        is_error: false,
                    },
                ],
            },
            meta: None,
        })
        .expect("results");
        for (call, effect) in [
            (
                "w1",
                CallEffect::FileWrite {
                    path: "notes/report.md".into(),
                    digest: None,
                    bytes: Some(4),
                },
            ),
            (
                "r1",
                CallEffect::FileRead {
                    path: "a.md".into(),
                    digest: None,
                    bytes: Some(4),
                },
            ),
        ] {
            log.append(Entry::new(
                log.tail_id().cloned(),
                EntryPayload::CallEffect(CallEffectRecord {
                    tool_use_id: call.into(),
                    effect,
                }),
            ))
            .expect("effect");
        }
        let timeline = snapshot("written", &log);
        let artifacts: Vec<_> = timeline
            .items
            .iter()
            .filter_map(|item| match &item.content {
                OutputContent::Artifact { artifact } => Some(artifact.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(artifacts.len(), 1, "{artifacts:?}");
        assert_eq!(artifacts[0].name, "report.md");
        assert_eq!(artifacts[0].path.as_deref(), Some("notes/report.md"));
        assert_eq!(artifacts[0].media_type.as_deref(), Some("text/markdown"));
    }

    #[test]
    fn sandbox_artifacts_rejoin_their_durable_result_turn() {
        let home = tempfile::tempdir().expect("temporary home");
        let events = home.path().join("sandbox/executions");
        std::fs::create_dir_all(&events).expect("execution directory");
        // An execution directory in the runtime root ends in
        // `<agent>/<execution>`, as the draft's name does.
        let scratch = home.path().join("runtime/executions/space/vak/call-1");
        std::fs::create_dir_all(&scratch).expect("scratch directory");
        let event = vak_tools::SandboxEvent::ArtifactGenerated {
            execution_id: "call-1".into(),
            path: ".vak/scratch/vak/call-1/report.html".into(),
            mime_type: "text/html".into(),
            size_bytes: 42,
        };
        let started = vak_tools::SandboxEvent::ExecutionStarted {
            execution_id: "call-1".into(),
            owner_session_id: Some("session-1".into()),
            tool: "bash".into(),
            code_preview: "generate report".into(),
            language: "sh".into(),
            scratch_dir: scratch.to_string_lossy().into_owned(),
        };
        vak_session::chain::RecordChain::at(events.join("session-1"))
            .append_all(&[
                serde_json::to_value(&started).expect("started json"),
                serde_json::to_value(&event).expect("event json"),
            ])
            .expect("event sidecar");

        let mut timeline = OutputTimeline::empty("session-1");
        timeline.items.push(OutputItem {
            placement: Default::default(),
            card_state: None,
            id: "tool-call-1".into(),
            timestamp: "2026-09-20T10:00:00+00:00".into(),
            turn_id: "turn-7".into(),
            role: OutputRole::Tool,
            kind: OutputKind::Progress,
            status: OutputStatus::Succeeded,
            outcome: None,
            content: OutputContent::Progress {
                label: "bash".into(),
                detail: None,
                percent: None,
            },
            provenance: Some(OutputProvenance {
                session_id: Some("session-1".into()),
                entry_id: Some("entry-1".into()),
                tool_call_id: Some("call-1".into()),
                source: Some("bash".into()),
                presentation_id: None,
            }),
            actions: Vec::new(),
            fallback_text: String::new(),
        });
        timeline.items.push(OutputItem {
            placement: Default::default(),
            card_state: None,
            id: "result-1".into(),
            timestamp: "2026-09-20T10:00:01+00:00".into(),
            turn_id: "turn-7".into(),
            role: OutputRole::Assistant,
            kind: OutputKind::Outcome,
            status: OutputStatus::Succeeded,
            outcome: Some(ResultOutcome {
                result_id: "result-1".into(),
                status: OutputStatus::Succeeded,
                completion: None,
                evidence_state: None,
                requirement_ids: Vec::new(),
                evidence_receipt_ids: Vec::new(),
                evidence: Vec::new(),
                human_review: None,
            }),
            content: OutputContent::Information {
                label: "Result".into(),
                detail: None,
            },
            provenance: None,
            actions: Vec::new(),
            fallback_text: "Result".into(),
        });

        super::append_sandbox_artifacts(&mut timeline, home.path(), "session-1");
        let artifact = timeline
            .items
            .iter()
            .find(|item| item.kind == OutputKind::Artifact)
            .expect("projected artifact");
        assert_eq!(artifact.turn_id, "turn-7");
        assert_eq!(artifact.timestamp, "2026-09-20T10:00:00+00:00");
        assert_eq!(
            artifact
                .outcome
                .as_ref()
                .map(|outcome| outcome.result_id.as_str()),
            Some("result-1")
        );
        assert!(artifact.actions.iter().any(|action| {
            action.verb == "review_draft"
                && action.data.get("execution_id").map(String::as_str) == Some("call-1")
        }));
        let OutputContent::Artifact { artifact } = &artifact.content else {
            panic!("artifact content");
        };
        assert_eq!(artifact.description, None, "no byte count in words");
        assert_eq!(artifact.size_bytes, Some(42));
        assert_eq!(
            artifact.status,
            Some(ArtifactStatus::Draft {
                version: 1,
                saved_as: None
            })
        );
    }

    /// A draft accepted through Review is recorded as a write of the folder
    /// file by the call that drafted it. That is the draft's acceptance, not
    /// a second file: the conversation shows one card for it.
    #[test]
    fn an_accepted_draft_is_one_card_not_two() {
        let home = tempfile::tempdir().expect("temporary home");
        let events = home.path().join("sandbox/executions");
        std::fs::create_dir_all(&events).expect("execution directory");
        let scratch = home.path().join("runtime/executions/space/vak/call-1");
        std::fs::create_dir_all(&scratch).expect("scratch directory");
        let draft = scratch.join("book.xlsx").to_string_lossy().into_owned();
        let started = vak_tools::SandboxEvent::ExecutionStarted {
            execution_id: "call-1".into(),
            owner_session_id: Some("session-1".into()),
            tool: "office_apply".into(),
            code_preview: String::new(),
            language: String::new(),
            scratch_dir: scratch.to_string_lossy().into_owned(),
        };
        let generated = vak_tools::SandboxEvent::ArtifactGenerated {
            execution_id: "call-1".into(),
            path: draft.clone(),
            mime_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".into(),
            size_bytes: 42,
        };
        vak_session::chain::RecordChain::at(events.join("session-1"))
            .append_all(&[
                serde_json::to_value(&started).expect("started json"),
                serde_json::to_value(&generated).expect("event json"),
            ])
            .expect("event sidecar");
        let mut timeline = OutputTimeline::empty("session-1");
        for (n, path) in [".vak/scratch/vak/call-1/book.xlsx", "book.xlsx"]
            .into_iter()
            .enumerate()
        {
            timeline.items.push(OutputItem {
                placement: Default::default(),
                card_state: None,
                id: format!("call-1-artifact-{n}"),
                timestamp: "2026-10-14T10:00:00+00:00".into(),
                turn_id: "turn-6".into(),
                role: OutputRole::Tool,
                kind: OutputKind::Artifact,
                status: OutputStatus::Succeeded,
                outcome: None,
                content: OutputContent::Artifact {
                    artifact: super::artifact_at(path),
                },
                provenance: Some(OutputProvenance {
                    session_id: Some("session-1".into()),
                    entry_id: Some("entry-1".into()),
                    tool_call_id: Some("call-1".into()),
                    source: Some("office_apply".into()),
                    presentation_id: None,
                }),
                actions: Vec::new(),
                fallback_text: path.into(),
            });
        }
        super::append_sandbox_artifacts(&mut timeline, home.path(), "session-1");
        let artifacts: Vec<_> = timeline
            .items
            .iter()
            .filter(|item| item.kind == OutputKind::Artifact)
            .collect();
        assert_eq!(artifacts.len(), 1, "{artifacts:?}");
        assert!(
            artifacts[0]
                .actions
                .iter()
                .any(|action| action.verb == "review_draft"),
            "the one card is the draft's, with its Review"
        );
    }

    /// A draft's status follows the durable records the way Review reads
    /// them: saved versions count up, an acceptance settles the round, an
    /// undo reopens it, and a version saved later starts a new round.
    #[test]
    fn draft_status_follows_saved_versions_and_acceptance() {
        let home = tempfile::tempdir().expect("temporary home");
        let events = home.path().join("sandbox/executions");
        std::fs::create_dir_all(&events).expect("execution directory");
        let scratch = home.path().join(".vak/scratch/vak/call-1");
        std::fs::create_dir_all(&scratch).expect("scratch directory");
        let started = vak_tools::SandboxEvent::ExecutionStarted {
            execution_id: "call-1".into(),
            owner_session_id: Some("session-1".into()),
            tool: "bash".into(),
            code_preview: "write page".into(),
            language: "sh".into(),
            scratch_dir: scratch.to_string_lossy().into_owned(),
        };
        let generated = vak_tools::SandboxEvent::ArtifactGenerated {
            execution_id: "call-1".into(),
            path: ".vak/scratch/vak/call-1/site/page.html".into(),
            mime_type: "text/html".into(),
            size_bytes: 120,
        };
        vak_session::chain::RecordChain::at(events.join("session-1"))
            .append_all(&[
                serde_json::to_value(&started).expect("started json"),
                serde_json::to_value(&generated).expect("event json"),
            ])
            .expect("write events");
        let candidate = |id: &str, session: &str| {
            serde_json::json!({"kind": "Candidate", "record": {
                "record_id": format!("record-{id}"), "session_id": session, "turn_id": "turn-1",
                "result_id": "result-1", "execution_id": "call-1", "environment_id": "env-1",
                "candidate_digest": "digest", "verified": true, "updated_at": "2026-09-26T00:00:00Z",
                "candidate": {"candidate_id": id, "source_root": "/saved", "destination_root": "/workspace",
                    "files": [{"path": "site/page.html", "candidate_hash": "h", "base_hash": null, "bytes": 120}]}
            }})
        };
        let promotion = |id: &str| {
            serde_json::json!({"kind": "Promotion", "record": {
                "record_id": format!("promotion-{id}"), "session_id": "session-1", "result_id": "result-1",
                "candidate_digest": "digest", "candidate_id": id, "updated_at": "2026-09-26T00:00:00Z",
                "receipt": {"candidate_id": id, "applied": [], "before_hashes": [], "after_hashes": [], "verification": []}
            }})
        };
        let undo = |id: &str| {
            serde_json::json!({"kind": "PromotionUndo", "record": {
                "record_id": format!("undo-{id}"), "session_id": "session-1", "candidate_id": id,
                "updated_at": "2026-09-26T00:00:00Z",
                "receipt": {"candidate_id": id, "restored": [], "verification": []}
            }})
        };
        let status_after = |records: &[serde_json::Value]| {
            let chain = home.path().join("sandbox/records");
            let _ = std::fs::remove_dir_all(&chain);
            vak_session::chain::RecordChain::at(&chain)
                .append_all(records)
                .expect("records");
            let mut timeline = OutputTimeline::empty("session-1");
            super::append_sandbox_artifacts(&mut timeline, home.path(), "session-1");
            let item = timeline
                .items
                .iter()
                .find(|item| item.kind == OutputKind::Artifact)
                .expect("projected artifact");
            let OutputContent::Artifact { artifact } = &item.content else {
                panic!("artifact content");
            };
            artifact.status.clone()
        };
        let saved = |id: &str| {
            Some(vak_delivery::VersionFile {
                version_id: id.into(),
                path: "site/page.html".into(),
            })
        };

        let mut records = vec![candidate("v1", "session-1"), candidate("x", "session-2")];
        assert_eq!(
            status_after(&records),
            Some(ArtifactStatus::Draft {
                version: 1,
                saved_as: saved("v1")
            }),
            "another conversation's version does not count"
        );
        records.push(candidate("v2", "session-1"));
        assert_eq!(
            status_after(&records),
            Some(ArtifactStatus::Draft {
                version: 2,
                saved_as: saved("v2")
            })
        );
        records.push(promotion("v2"));
        assert_eq!(
            status_after(&records),
            Some(ArtifactStatus::Accepted {
                version: 2,
                saved_as: saved("v2")
            })
        );
        records.push(undo("v2"));
        assert_eq!(
            status_after(&records),
            Some(ArtifactStatus::Draft {
                version: 2,
                saved_as: saved("v2")
            }),
            "undoing the acceptance reopens the draft"
        );
        records.push(promotion("v1"));
        records.push(candidate("v3", "session-1"));
        assert_eq!(
            status_after(&records),
            Some(ArtifactStatus::Draft {
                version: 3,
                saved_as: saved("v3")
            }),
            "a version saved after an acceptance starts a new round"
        );

        let chain = home.path().join("sandbox/records");
        let _ = std::fs::remove_dir_all(&chain);
        vak_session::chain::RecordChain::at(&chain)
            .append(&serde_json::json!({"kind": "NotARecord"}))
            .expect("records");
        let mut timeline = OutputTimeline::empty("session-1");
        super::append_sandbox_artifacts(&mut timeline, home.path(), "session-1");
        let OutputContent::Artifact { artifact } = &timeline.items[0].content else {
            panic!("artifact content");
        };
        assert_eq!(
            artifact.status, None,
            "unreadable records leave the status unknown"
        );
    }

    #[test]
    fn direct_write_artifact_merges_sidecar_without_false_review_action() {
        let home = tempfile::tempdir().expect("temporary home");
        let events = home.path().join("sandbox/executions");
        std::fs::create_dir_all(&events).expect("execution directory");
        let observed = vak_tools::SandboxEvent::ArtifactGenerated {
            execution_id: "write-1".into(),
            path: "page.html".into(),
            mime_type: "text/html".into(),
            size_bytes: 21,
        };
        vak_session::chain::RecordChain::at(events.join("session-1"))
            .append(&observed)
            .expect("event sidecar");
        let mut timeline = OutputTimeline::empty("session-1");
        timeline.items.push(OutputItem {
            placement: Default::default(),
            card_state: None,
            id: "write-1-artifact".into(),
            timestamp: "2026-09-20T10:00:00+00:00".into(),
            turn_id: "turn-1".into(),
            role: OutputRole::Tool,
            kind: OutputKind::Artifact,
            status: OutputStatus::Succeeded,
            outcome: None,
            content: OutputContent::Artifact {
                artifact: vak_delivery::ArtifactRef {
                    name: "page.html".into(),
                    path: Some("page.html".into()),
                    media_type: Some("text/html".into()),
                    description: None,
                    size_bytes: None,
                    status: Some(ArtifactStatus::InFolder),
                },
            },
            provenance: Some(OutputProvenance {
                session_id: Some("session-1".into()),
                entry_id: Some("entry-1".into()),
                tool_call_id: Some("write-1".into()),
                source: Some("write".into()),
                presentation_id: None,
            }),
            actions: Vec::new(),
            fallback_text: "page.html".into(),
        });

        super::append_sandbox_artifacts(&mut timeline, home.path(), "session-1");

        let artifacts = timeline
            .items
            .iter()
            .filter(|item| item.kind == OutputKind::Artifact)
            .collect::<Vec<_>>();
        assert_eq!(artifacts.len(), 1);
        assert!(
            artifacts[0]
                .actions
                .iter()
                .any(|action| action.verb == "open_artifact")
        );
        assert!(
            !artifacts[0]
                .actions
                .iter()
                .any(|action| action.verb == "review_draft")
        );
        let OutputContent::Artifact { artifact } = &artifacts[0].content else {
            panic!("artifact content");
        };
        assert_eq!(artifact.status, Some(ArtifactStatus::InFolder));
        assert_eq!(artifact.description, None);
        assert_eq!(artifact.size_bytes, Some(21));
    }

    #[test]
    fn outcome_evaluation_exposes_review_actions() {
        let item = activity_item(
            "session-1",
            "activity-1",
            "2026-01-01T00:00:00Z",
            "turn-1",
            Some("turn-entry-1"),
            &ActivityRecord {
                activity_id: "evaluation-1".into(),
                kind: ActivityKind::Diagnostic,
                status: ActivityStatus::Succeeded,
                label: "Outcome evaluation".into(),
                detail: Some("partial".into()),
                data: BTreeMap::new(),
            },
        );
        assert_eq!(item.actions.len(), 3);
        assert!(
            item.actions
                .iter()
                .all(|action| action.verb == "record_outcome_review")
        );
    }

    #[test]
    fn repeated_writes_keep_one_latest_artifact_per_turn_and_path() {
        let artifact = |id: &str, turn: &str, path: &str| OutputItem {
            placement: Default::default(),
            card_state: None,
            id: id.into(),
            timestamp: "2026-09-23T00:00:00Z".into(),
            turn_id: turn.into(),
            role: OutputRole::Tool,
            kind: OutputKind::Artifact,
            status: OutputStatus::Succeeded,
            outcome: None,
            content: OutputContent::Artifact {
                artifact: vak_delivery::ArtifactRef {
                    name: path.into(),
                    path: Some(path.into()),
                    media_type: Some("text/csv".into()),
                    description: None,
                    size_bytes: None,
                    status: None,
                },
            },
            provenance: None,
            actions: Vec::new(),
            fallback_text: path.into(),
        };
        let mut items = vec![
            artifact("first", "turn-1", "report.csv"),
            artifact("other-dir", "turn-1", "sub/report.csv"),
            artifact("other-turn", "turn-2", "report.csv"),
            artifact("last", "turn-1", "report.csv"),
        ];
        super::deduplicate_file_artifacts(&mut items);
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["other-dir", "other-turn", "last"]
        );
    }

    #[test]
    fn scratch_artifacts_dedupe_by_relative_path_per_turn() {
        let artifact = |id: &str, path: &str, execution: &str| OutputItem {
            placement: Default::default(),
            card_state: None,
            id: id.into(),
            timestamp: "2026-09-23T00:00:00Z".into(),
            turn_id: "turn-1".into(),
            role: OutputRole::Tool,
            kind: OutputKind::Artifact,
            status: OutputStatus::Succeeded,
            outcome: None,
            content: OutputContent::Artifact {
                artifact: vak_delivery::ArtifactRef {
                    name: "report.xlsx".into(),
                    path: Some(path.into()),
                    media_type: Some(
                        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".into(),
                    ),
                    description: None,
                    size_bytes: None,
                    status: None,
                },
            },
            provenance: Some(OutputProvenance {
                session_id: Some("session-1".into()),
                entry_id: None,
                tool_call_id: Some(execution.into()),
                source: Some("sandbox_artifact".into()),
                presentation_id: None,
            }),
            actions: Vec::new(),
            fallback_text: path.into(),
        };
        let mut items = vec![
            artifact("old", "/workspace/.vak/scratch/run-a/report.xlsx", "run-a"),
            artifact(
                "other-dir",
                "/workspace/.vak/scratch/run-b/archive/report.xlsx",
                "run-b",
            ),
            artifact(
                "latest",
                "/workspace/.vak/scratch/run-b/report.xlsx",
                "run-b",
            ),
        ];
        let roots = HashMap::from([
            ("run-a".into(), "/workspace/.vak/scratch/run-a".into()),
            ("run-b".into(), "/workspace/.vak/scratch/run-b".into()),
        ]);
        super::deduplicate_file_artifacts_with_scratch(&mut items, &roots);
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["other-dir", "latest"]
        );
    }

    #[test]
    fn snapshot_is_outcome_first_and_deduplicates_activity_transitions() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let mut log = SessionLog::create(
            dir.path().join("presentation.jsonl"),
            SessionHeader {
                space: None,
                run: None,
                cause: None,
                agent: None,
                session_id: "session-1".into(),
                created_at: chrono::Utc::now(),
                cwd: PathBuf::from("/tmp/project"),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
                conversation: None,
                contract: FrozenContract {
                    app_version: "test".into(),
                    provider: "test".into(),
                    model: "test".into(),
                    route_ladder: Vec::new(),
                    route_objective: String::new(),
                    route_annotations: Vec::new(),
                    system_prompt: String::new(),
                    permission_mode: "read-only".into(),
                    capabilities: Vec::new(),
                    prompt_layers: Vec::new(),
                },
            },
        )
        .expect("create session");
        log.append_message(MessageRecord {
            message: Message::user_text("Build the report"),
            meta: None,
        })
        .expect("append user");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![
                ContentBlock::ToolUse {
                    id: "tool-1".into(),
                    name: "write".into(),
                    input: serde_json::json!({"path": "/tmp/report.md"}),
                },
                ContentBlock::text(
                    "## Done\n\nThe report is ready. See https://example.com/report",
                ),
            ]),
            meta: None,
        })
        .expect("append assistant");
        // What the loop records for a call that wrote a file.
        log.append(vak_session::types::Entry::new(
            log.tail_id().cloned(),
            vak_session::types::EntryPayload::CallEffect(vak_session::types::CallEffectRecord {
                tool_use_id: "tool-1".into(),
                effect: vak_session::types::CallEffect::FileWrite {
                    path: "/tmp/report.md".into(),
                    digest: None,
                    bytes: None,
                },
            }),
        ))
        .expect("append write effect");
        for status in [ActivityStatus::Pending, ActivityStatus::Succeeded] {
            log.append_activity(ActivityRecord {
                activity_id: "approval-1".into(),
                kind: ActivityKind::Approval,
                status,
                label: "Approval".into(),
                detail: Some("Write report".into()),
                data: BTreeMap::from([
                    ("request_id".into(), "request-1".into()),
                    ("tool".into(), "write".into()),
                    ("args_json".into(), "{}".into()),
                ]),
            })
            .expect("append approval state");
        }
        log.append_activity(ActivityRecord {
            activity_id: "evaluation-1".into(),
            kind: ActivityKind::Diagnostic,
            status: ActivityStatus::Succeeded,
            label: "Outcome evaluation".into(),
            detail: Some("primary deliverable: produced".into()),
            data: BTreeMap::from([
                ("status".into(), "produced".into()),
                ("completion".into(), "unknown".into()),
                ("evaluation".into(), "[]".into()),
                ("evidence_receipts".into(), String::new()),
            ]),
        })
        .expect("append outcome evaluation");
        log.append_activity(ActivityRecord {
            activity_id: "review-1".into(),
            kind: ActivityKind::Diagnostic,
            status: ActivityStatus::Succeeded,
            label: "Outcome review".into(),
            detail: None,
            data: BTreeMap::from([(String::from("verdict"), String::from("accepted"))]),
        })
        .expect("append outcome review");
        log.append_activity(ActivityRecord {
            activity_id: "review-2".into(),
            kind: ActivityKind::Diagnostic,
            status: ActivityStatus::Succeeded,
            label: "Outcome review".into(),
            detail: Some("superseding review".into()),
            data: BTreeMap::from([(String::from("verdict"), String::from("needs_work"))]),
        })
        .expect("append superseding outcome review");

        let first = snapshot("session-1", &log);
        let second = snapshot("session-1", &log);
        assert_eq!(first, second);
        assert_eq!(
            first
                .items
                .iter()
                .filter(|item| item.id == "approval-1")
                .count(),
            1
        );
        let approval = first
            .items
            .iter()
            .find(|item| item.id == "approval-1")
            .expect("approval item");
        assert_eq!(approval.status, OutputStatus::Succeeded);
        assert!(
            first
                .items
                .iter()
                .any(|item| item.kind == OutputKind::Artifact)
        );
        let outcome = first
            .items
            .iter()
            .find(|item| item.kind == OutputKind::Outcome)
            .expect("assistant outcome");
        let OutputContent::Document { document } = &outcome.content else {
            panic!("assistant outcome must contain its document");
        };
        assert_eq!(
            document.source_markdown,
            "## Done\n\nThe report is ready. See https://example.com/report"
        );
        assert_eq!(document.metadata.get("recipe_id").map(String::as_str), None);
        assert_eq!(document.metadata.get("renderer").map(String::as_str), None);
        assert_eq!(
            document
                .metadata
                .get("outcome_review_verdict")
                .map(String::as_str),
            Some("needs_work")
        );
        assert_eq!(
            link_cards(&first),
            0,
            "a URL in the answer's text is not a card"
        );
    }

    #[test]
    fn recovered_tool_failure_stays_in_activity_without_attention_banner() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let mut log = SessionLog::create(
            dir.path().join("presentation.jsonl"),
            SessionHeader {
                space: None,
                run: None,
                cause: None,
                agent: None,
                session_id: "session-2".into(),
                created_at: chrono::Utc::now(),
                cwd: PathBuf::from("/tmp/project"),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
                conversation: None,
                contract: FrozenContract {
                    app_version: "test".into(),
                    provider: "test".into(),
                    model: "test".into(),
                    route_ladder: Vec::new(),
                    route_objective: String::new(),
                    route_annotations: Vec::new(),
                    system_prompt: String::new(),
                    permission_mode: "read-only".into(),
                    capabilities: Vec::new(),
                    prompt_layers: Vec::new(),
                },
            },
        )
        .expect("create session");
        log.append_message(MessageRecord {
            message: Message::user_text("Search the web"),
            meta: None,
        })
        .expect("append user");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::ToolUse {
                id: "tool-failed".into(),
                name: "mcp".into(),
                input: serde_json::json!({"tool": "search"}),
            }]),
            meta: None,
        })
        .expect("append failed call");
        log.append_message(MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "tool-failed".into(),
                    content: "Unknown tool: search".into(),
                    is_error: true,
                }],
            },
            meta: None,
        })
        .expect("append failed result");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text(
                "Search completed after recovery",
            )]),
            meta: None,
        })
        .expect("append outcome");
        log.append_activity(ActivityRecord {
            activity_id: "run-2".into(),
            kind: ActivityKind::Run,
            status: ActivityStatus::Succeeded,
            label: "Run finished".into(),
            detail: Some("completed".into()),
            data: BTreeMap::new(),
        })
        .expect("append run status");

        let timeline = snapshot("session-2", &log);
        assert!(
            !timeline
                .items
                .iter()
                .any(|item| item.kind == OutputKind::Error)
        );
        let recovered = timeline
            .items
            .iter()
            .find(|item| item.id == "tool-failed")
            .expect("recovered tool item");
        assert_eq!(recovered.kind, OutputKind::Progress);
        assert_eq!(recovered.status, OutputStatus::Failed);
        assert!(recovered.fallback_text.contains("Unknown tool"));
        let outcome = timeline
            .items
            .iter()
            .find(|item| item.kind == OutputKind::Outcome)
            .expect("assistant outcome");
        assert_eq!(
            outcome
                .provenance
                .as_ref()
                .and_then(|provenance| provenance.tool_call_id.as_deref()),
            Some("tool-failed")
        );
    }

    #[test]
    fn completion_status_requires_exact_complete_verdict() {
        assert_eq!(super::status_for_completion(None), OutputStatus::Succeeded);
        assert_eq!(
            super::status_for_completion(Some("complete")),
            OutputStatus::Succeeded
        );
        assert_eq!(
            super::status_for_completion(Some("incomplete")),
            OutputStatus::Partial
        );
        assert_eq!(
            super::status_for_completion(Some("unknown")),
            OutputStatus::Partial
        );
    }

    #[test]
    fn authored_marker_examples_survive_the_actual_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "literal-content");
        let literal = "Outcome: completed\nSurface: desktop app\n[recovery] is a literal marker\n<intent>example</intent>\ncompleted";
        log.append_message(MessageRecord {
            message: Message::user_text(literal),
            meta: None,
        })
        .expect("request");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text(literal)]),
            meta: None,
        })
        .expect("reply");
        let timeline = snapshot("literal-content", &log);
        let documents: Vec<_> = timeline
            .items
            .iter()
            .filter_map(|item| match &item.content {
                OutputContent::Document { document } => Some(document.source_markdown.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            documents.iter().filter(|text| **text == literal).count() >= 2,
            "{documents:?}"
        );
    }

    #[test]
    fn synthetic_stop_messages_do_not_increment_turn_or_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = SessionLog::create(
            dir.path().join("synthetic-turns.jsonl"),
            SessionHeader {
                space: None,
                run: None,
                cause: None,
                agent: None,
                session_id: "synthetic-turns".into(),
                created_at: chrono::Utc::now(),
                cwd: PathBuf::from("/tmp/project"),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
                conversation: None,
                contract: FrozenContract {
                    app_version: "test".into(),
                    provider: "test".into(),
                    model: "test".into(),
                    route_ladder: Vec::new(),
                    route_objective: String::new(),
                    route_annotations: Vec::new(),
                    system_prompt: String::new(),
                    permission_mode: "read-only".into(),
                    capabilities: Vec::new(),
                    prompt_layers: Vec::new(),
                },
            },
        )
        .expect("create session");

        // Real user turn 1
        log.append_message(MessageRecord {
            message: Message::user_text("User query 1"),
            meta: None,
        })
        .expect("append user message");

        log.append_message(MessageRecord {
            message: Message {
                role: Role::Assistant,
                content: vec![ContentBlock::Text {
                    text: "Assistant answer 1".into(),
                }],
            },
            meta: None,
        })
        .expect("append assistant message");

        // Synthetic stop-hook nudge (should NOT increment turn count)
        log.append_message(MessageRecord {
            message: Message::user_text(vak_control::render(
                &vak_control::Finding::StopHook {
                    reason: "hook said continue".into(),
                },
                "[stop-hook]",
            )),
            meta: Some(vak_session::MessageMeta {
                control: Some(vak_intent::control::ControlKind::StopHook),
                ..Default::default()
            }),
        })
        .expect("append stop-hook message");

        log.append_message(MessageRecord {
            message: Message {
                role: Role::Assistant,
                content: vec![ContentBlock::Text {
                    text: "Assistant answer 2".into(),
                }],
            },
            meta: None,
        })
        .expect("append assistant message 2");

        // Synthetic repair directive nudge (should NOT increment turn count)
        log.append_message(MessageRecord {
            message: Message::user_text("The run is stuck on correctable tool failures..."),
            meta: Some(vak_session::MessageMeta {
                control: Some(vak_intent::control::ControlKind::RepairDirective),
                ..Default::default()
            }),
        })
        .expect("append repair directive message");

        log.append_message(MessageRecord {
            message: Message {
                role: Role::Assistant,
                content: vec![ContentBlock::Text {
                    text: "Assistant answer 3".into(),
                }],
            },
            meta: None,
        })
        .expect("append assistant message 3");

        let timeline = snapshot("synthetic-turns", &log);
        // There should only be 1 user message projected, not 3
        let user_items: Vec<_> = timeline
            .items
            .iter()
            .filter(|i| i.role == vak_delivery::OutputRole::User)
            .collect();
        assert_eq!(
            user_items.len(),
            1,
            "synthetic stop/repair messages must not be projected as user item"
        );
        assert_eq!(user_items[0].turn_id, "turn-1");
    }

    /// A second same-type card with NO repair nudge in between is a
    /// legitimate distinct card (e.g. "chart A, then chart B") and must
    /// NOT be collapsed.
    #[test]
    fn two_same_type_cards_with_no_repair_nudge_both_survive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = SessionLog::create(
            dir.path().join("two-charts.jsonl"),
            SessionHeader {
                space: None,
                run: None,
                cause: None,
                agent: None,
                session_id: "two-charts".into(),
                created_at: chrono::Utc::now(),
                cwd: PathBuf::from("/tmp/project"),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
                conversation: None,
                contract: FrozenContract {
                    app_version: "test".into(),
                    provider: "test".into(),
                    model: "test".into(),
                    route_ladder: Vec::new(),
                    route_objective: String::new(),
                    route_annotations: Vec::new(),
                    system_prompt: String::new(),
                    permission_mode: "read-only".into(),
                    capabilities: Vec::new(),
                    prompt_layers: Vec::new(),
                },
            },
        )
        .expect("create session");

        log.append_message(MessageRecord {
            message: Message::user_text("show me revenue and cost charts"),
            meta: None,
        })
        .expect("append user message");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::ToolUse {
                id: "call-1".into(),
                name: "emit_chart_card".into(),
                input: serde_json::json!({"semantic_type":"chart","chart_type":"line","data":[{"series":"s","x":"Mon","y":1}],"accessible_summary":"revenue"}),
            }]),
            meta: None,
        })
        .expect("append call 1");
        log.append_message(MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "call-1".into(),
                    content: "Card displayed to the user (chart).".into(),
                    is_error: false,
                }],
            },
            meta: None,
        })
        .expect("append result 1");
        append_presentation_for_call(
            &mut log,
            "emit_chart_card",
            "call-1",
            &serde_json::json!({"semantic_type":"chart","chart_type":"line","data":[{"series":"s","x":"Mon","y":1}],"accessible_summary":"revenue"}),
        );
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text(
                "Initial chart draft that the additive completeness check replaces.",
            )]),
            meta: None,
        })
        .expect("append first draft text");
        log.append_message(MessageRecord::control(
            vak_intent::control::ControlKind::CardCompletenessCheck,
            "[card-completeness-check]: Add any missing outcomes.",
        ))
        .expect("append additive completeness check");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::ToolUse {
                id: "call-2".into(),
                name: "emit_chart_card".into(),
                input: serde_json::json!({"semantic_type":"chart","chart_type":"line","data":[{"series":"s","x":"Mon","y":1}],"accessible_summary":"cost"}),
            }]),
            meta: None,
        })
        .expect("append call 2");
        log.append_message(MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "call-2".into(),
                    content: "Card displayed to the user (chart).".into(),
                    is_error: false,
                }],
            },
            meta: None,
        })
        .expect("append result 2");
        append_presentation_for_call(
            &mut log,
            "emit_chart_card",
            "call-2",
            &serde_json::json!({"semantic_type":"chart","chart_type":"line","data":[{"series":"s","x":"Mon","y":1}],"accessible_summary":"cost"}),
        );

        // A later user turn may intentionally ask for the same card again.
        // Per-turn duplicate suppression must not hide it from the timeline.
        let repeated_chart = serde_json::json!({"semantic_type":"chart","chart_type":"line","data":[{"series":"s","x":"Mon","y":1}],"accessible_summary":"revenue"});
        log.append_message(MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![ContentBlock::image_base64("image/png", "synthetic")],
            },
            meta: None,
        })
        .expect("append image-only second user turn");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::ToolUse {
                id: "call-3".into(),
                name: "emit_chart_card".into(),
                input: repeated_chart.clone(),
            }]),
            meta: None,
        })
        .expect("append call 3");
        log.append_message(MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "call-3".into(),
                    content: "Card displayed to the user (chart).".into(),
                    is_error: false,
                }],
            },
            meta: None,
        })
        .expect("append result 3");
        append_presentation_for_call(&mut log, "emit_chart_card", "call-3", &repeated_chart);

        let timeline = snapshot("two-charts", &log);
        let chart_items: Vec<_> = timeline
            .items
            .iter()
            .filter(|item| {
                matches!(&item.content, OutputContent::Structured { output } if output.semantic_type == "chart")
            })
            .collect();
        assert_eq!(
            chart_items.len(),
            3,
            "two same-type cards in one turn and an identical card in a later turn must all survive: {chart_items:?}"
        );
        assert!(!timeline.items.iter().any(|item| {
            match &item.content {
                OutputContent::Document { document } => document
                    .source_markdown
                    .contains("Initial chart draft that the additive completeness check replaces."),
                OutputContent::Outcome {
                    document: Some(document),
                    ..
                } => document
                    .source_markdown
                    .contains("Initial chart draft that the additive completeness check replaces."),
                _ => false,
            }
        }));
    }

    /// The live bug: a research card over the tool framework's ~2000-char
    /// line limit had its recorded RESULT truncated mid-JSON, so no card was
    /// produced and the user saw prose. The card now comes from the call's
    /// own (untruncated) arguments; the result is only an ack.
    #[test]
    fn a_card_larger_than_the_result_line_limit_still_renders_from_the_call() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = SessionLog::create(
            dir.path().join("big-card.jsonl"),
            SessionHeader {
                space: None,
                run: None,
                cause: None,
                agent: None,
                session_id: "big-card".into(),
                created_at: chrono::Utc::now(),
                cwd: PathBuf::from("/tmp/project"),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
                conversation: None,
                contract: FrozenContract {
                    app_version: "test".into(),
                    provider: "test".into(),
                    model: "test".into(),
                    route_ladder: Vec::new(),
                    route_objective: String::new(),
                    route_annotations: Vec::new(),
                    system_prompt: String::new(),
                    permission_mode: "read-only".into(),
                    capabilities: Vec::new(),
                    prompt_layers: Vec::new(),
                },
            },
        )
        .expect("create session");
        log.append_message(MessageRecord {
            message: Message::user_text("summarize last week's market"),
            meta: None,
        })
        .expect("user");
        let sources: Vec<_> = (0..12)
            .map(|i| {
                serde_json::json!({
                    "title": format!("Reuters {i}"),
                    "url": format!("https://example.com/{i}"),
                    "finding": format!("Takeaway {i}: {}", "detail ".repeat(40))
                })
            })
            .collect();
        let input = serde_json::json!({
            "sources": sources
        });
        assert!(
            input.to_string().len() > 3000,
            "fixture must exceed the limit"
        );
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::ToolUse {
                id: "call-r".into(),
                name: "emit_sources_card".into(),
                input: input.clone(),
            }]),
            meta: None,
        })
        .expect("call");
        log.append_message(MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "call-r".into(),
                    content: "Shown to the user (sources).".into(),
                    is_error: false,
                }],
            },
            meta: None,
        })
        .expect("result");
        append_presentation_for_call(&mut log, "emit_sources_card", "call-r", &input);
        let timeline = snapshot("big-card", &log);
        assert!(
            timeline.items.iter().any(|item| matches!(&item.content,
                OutputContent::Structured { output } if output.semantic_type == "sources")),
            "the research card must render from the call arguments"
        );
    }

    /// A card's `provenance.presentation_id` is the `Presentation` ledger
    /// entry's OWN id (docs/design/68-context-engine.md §10), not the id of
    /// the message entry the `emit_*_card` tool call rode in on — those two
    /// entries are different chain entries, and the client's
    /// `/presentation/feedback` and `/presentation/select` calls key on the
    /// former.
    #[test]
    fn card_provenance_carries_the_presentations_own_entry_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = SessionLog::create(
            dir.path().join("card-id.jsonl"),
            SessionHeader {
                space: None,
                run: None,
                cause: None,
                agent: None,
                session_id: "card-id".into(),
                created_at: chrono::Utc::now(),
                cwd: PathBuf::from("/tmp/project"),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
                conversation: None,
                contract: FrozenContract {
                    app_version: "test".into(),
                    provider: "test".into(),
                    model: "test".into(),
                    route_ladder: Vec::new(),
                    route_objective: String::new(),
                    route_annotations: Vec::new(),
                    system_prompt: String::new(),
                    permission_mode: "read-only".into(),
                    capabilities: Vec::new(),
                    prompt_layers: Vec::new(),
                },
            },
        )
        .expect("create session");
        log.append_message(MessageRecord {
            message: Message::user_text("what's the temperature"),
            meta: None,
        })
        .expect("user");
        let input = serde_json::json!({
            "readings": [{"label": "Temperature", "value": "25", "unit": "C"}]
        });
        let call_entry = log
            .append_message(MessageRecord {
                message: Message::assistant(vec![ContentBlock::ToolUse {
                    id: "call-m".into(),
                    name: "emit_stats_card".into(),
                    input: input.clone(),
                }]),
                meta: None,
            })
            .expect("call");
        log.append_message(MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "call-m".into(),
                    content: "Card displayed to the user (metric).".into(),
                    is_error: false,
                }],
            },
            meta: None,
        })
        .expect("result");
        append_presentation_for_call(&mut log, "emit_stats_card", "call-m", &input);
        let (presentation_entry_id, _) = log
            .presentations()
            .into_iter()
            .next()
            .expect("one presentation entry was written");
        assert_ne!(
            presentation_entry_id, call_entry.id,
            "the Presentation entry must be its own chain entry, distinct \
             from the message carrying the tool_use block"
        );

        let timeline = snapshot("card-id", &log);
        let card = timeline
            .items
            .iter()
            .find(|item| item.kind == OutputKind::Card)
            .expect("a metric card must be projected");
        assert_eq!(
            card.provenance
                .as_ref()
                .and_then(|p| p.presentation_id.clone()),
            Some(presentation_entry_id),
            "the card's presentation_id must be the Presentation entry's \
             own id, not the tool_use message's entry id"
        );
    }

    #[test]
    fn a_runtime_nudge_is_never_projected_as_a_user_message_whatever_its_text() {
        use vak_intent::control::ControlKind;
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = SessionLog::create(
            dir.path().join("nudge.jsonl"),
            SessionHeader {
                space: None,
                run: None,
                cause: None,
                agent: None,
                session_id: "nudge".into(),
                created_at: chrono::Utc::now(),
                cwd: PathBuf::from("/tmp/project"),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
                conversation: None,
                contract: FrozenContract {
                    app_version: "test".into(),
                    provider: "test".into(),
                    model: "test".into(),
                    route_ladder: Vec::new(),
                    route_objective: String::new(),
                    route_annotations: Vec::new(),
                    system_prompt: String::new(),
                    permission_mode: "read-only".into(),
                    capabilities: Vec::new(),
                    prompt_layers: Vec::new(),
                },
            },
        )
        .expect("create");
        log.append_message(MessageRecord {
            message: Message::user_text("what is the weather"),
            meta: None,
        })
        .expect("user");
        for kind in ControlKind::ALL {
            // No marker in the text at all: only the tag says it is runtime-authored.
            log.append_message(MessageRecord::control(kind, "please redo that answer"))
                .expect("nudge");
        }
        // Text that merely LOOKS like a marker, from the user, stays the user's.
        log.append_message(MessageRecord {
            message: Message::user_text("[fence-check]: my own note"),
            meta: None,
        })
        .expect("look-alike");
        let timeline = snapshot("nudge", &log);
        let users: Vec<_> = timeline
            .items
            .iter()
            .filter(|i| i.role == vak_delivery::OutputRole::User)
            .collect();
        assert_eq!(
            users.len(),
            2,
            "the real question and the user's look-alike, no nudges: {:?}",
            users.iter().map(|i| &i.fallback_text).collect::<Vec<_>>()
        );
        let turns: std::collections::BTreeSet<_> =
            users.iter().map(|i| i.turn_id.clone()).collect();
        assert_eq!(
            turns.len(),
            2,
            "nudges must not start turns; the look-alike does"
        );
    }

    /// The projection's user items carry the ledger entry id, which is what
    /// the client pairs a chat turn with instead of counting turns.
    #[test]
    fn a_user_item_carries_the_ledger_entry_id_of_its_message() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = SessionLog::create(
            dir.path().join("ident.jsonl"),
            SessionHeader {
                space: None,
                run: None,
                cause: None,
                agent: None,
                session_id: "ident".into(),
                created_at: chrono::Utc::now(),
                cwd: PathBuf::from("/tmp/project"),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
                conversation: None,
                contract: FrozenContract {
                    app_version: "test".into(),
                    provider: "test".into(),
                    model: "test".into(),
                    route_ladder: Vec::new(),
                    route_objective: String::new(),
                    route_annotations: Vec::new(),
                    system_prompt: String::new(),
                    permission_mode: "read-only".into(),
                    capabilities: Vec::new(),
                    prompt_layers: Vec::new(),
                },
            },
        )
        .expect("create");
        log.append_message(MessageRecord {
            message: Message::user_text("first"),
            meta: None,
        })
        .expect("u1");
        log.append_message(MessageRecord::control(
            vak_intent::control::ControlKind::GroundingCheck,
            "redo",
        ))
        .expect("n");
        log.append_message(MessageRecord {
            message: Message::user_text("second"),
            meta: None,
        })
        .expect("u2");
        let transcript = log.derive_transcript();
        let real: Vec<_> = transcript.iter().filter(|t| t.control.is_none()).collect();
        let timeline = snapshot("ident", &log);
        for item in &real {
            let text = item.message.text_content();
            let user_item = timeline
                .items
                .iter()
                .find(|i| i.role == vak_delivery::OutputRole::User && i.fallback_text == text)
                .expect("projected");
            assert_eq!(
                user_item
                    .provenance
                    .as_ref()
                    .and_then(|p| p.entry_id.as_deref()),
                Some(item.entry_id.as_str()),
                "pairing key must be the transcript's entry id for {text:?}"
            );
        }
    }

    /// Appends `padding` to the longest string in `value` (never
    /// `semantic_type`; a Markdown table gains a padded row), so a padded
    /// fixture still carries only the fields its card accepts.
    fn pad_longest_text(value: &mut serde_json::Value, padding: &str) -> bool {
        fn longest<'a>(value: &'a mut serde_json::Value, best: &mut Option<&'a mut String>) {
            match value {
                serde_json::Value::Object(fields) => {
                    for (key, field) in fields.iter_mut() {
                        if key == "semantic_type" {
                            continue;
                        }
                        if let serde_json::Value::String(text) = field {
                            if best.as_ref().is_none_or(|b| b.len() < text.len()) {
                                *best = Some(text);
                            }
                        } else {
                            longest(field, best);
                        }
                    }
                }
                serde_json::Value::Array(items) => {
                    for item in items {
                        if let serde_json::Value::String(text) = item {
                            if best.as_ref().is_none_or(|b| b.len() < text.len()) {
                                *best = Some(text);
                            }
                        } else {
                            longest(item, best);
                        }
                    }
                }
                _ => {}
            }
        }
        let mut best = None;
        longest(value, &mut best);
        best.map(|text| {
            if text.trim_end().ends_with('|') {
                let columns = text
                    .lines()
                    .next()
                    .map_or(1, |row| row.matches('|').count().saturating_sub(1));
                let cells = vec![padding.trim(); columns.max(1)].join(" | ");
                text.push_str(&format!("\n| {cells} |"));
            } else {
                text.push_str(padding);
            }
        })
        .is_some()
    }

    /// Not per-type and not per-size: every type the emit tools support, at a
    /// normal size and padded (one of its own text fields) far past the tool framework's result line
    /// limit, must come out of the real `snapshot()` as exactly one card of
    /// that type — from the call arguments alone, with only an ack as result.
    #[test]
    fn every_supported_type_renders_through_the_full_path_at_any_size() {
        let cases = vak_core::presentation_tools::conformance_cases();
        let emitters = vak_core::presentation_tools::EmitCardTool::all();
        for emitter in &emitters {
            let name = vak_tools::Tool::name(emitter);
            // A card filled from an ability's answer (docs/design/93 R5) has
            // no arguments that show it alone: it is recorded from the
            // arguments the loop filled, and vak-agent's card_gates test
            // follows it from the ability's call.
            if vak_tools::Tool::data_of_call(emitter).is_some() {
                continue;
            }
            assert!(
                cases.iter().any(|(tool, _, _)| *tool == name),
                "no case for block {name}"
            );
        }
        let mut failures = Vec::new();
        for (tool, semantic_type, args) in cases {
            let mut normalized_args = args.clone();
            if let Some(emitter) = emitters
                .iter()
                .find(|emitter| vak_tools::Tool::name(*emitter) == tool)
            {
                vak_tools::Tool::fill_defaults(emitter, &mut normalized_args);
                if let Err(error) =
                    vak_tools::validate_input(&vak_tools::Tool::schema(emitter), &normalized_args)
                {
                    failures.push(format!("{semantic_type} via {tool}: pre-execution schema rejected normalized call: {error}"));
                    continue;
                }
            }
            for padded in [false, true] {
                let mut args = normalized_args.clone();
                if padded {
                    assert!(
                        pad_longest_text(&mut args, &" p".repeat(3000)),
                        "{semantic_type}: a fixture with no text to pad"
                    );
                }
                let dir = tempfile::tempdir().expect("tempdir");
                let mut log = SessionLog::create(
                    dir.path().join("t.jsonl"),
                    SessionHeader {
                        space: None,
                        run: None,
                        cause: None,
                        agent: None,
                        session_id: "conformance".into(),
                        created_at: chrono::Utc::now(),
                        cwd: PathBuf::from("/tmp/project"),
                        parent_session_id: None,
                        contract_id: None,
                        work_item_id: None,
                        conversation: None,
                        contract: FrozenContract {
                            app_version: "test".into(),
                            provider: "test".into(),
                            model: "test".into(),
                            route_ladder: Vec::new(),
                            route_objective: String::new(),
                            route_annotations: Vec::new(),
                            system_prompt: String::new(),
                            permission_mode: "read-only".into(),
                            capabilities: Vec::new(),
                            prompt_layers: Vec::new(),
                        },
                    },
                )
                .expect("create");
                log.append_message(MessageRecord {
                    message: Message::user_text("go"),
                    meta: None,
                })
                .expect("user");
                let call_args = args.clone();
                log.append_message(MessageRecord {
                    message: Message::assistant(vec![ContentBlock::ToolUse {
                        id: "c1".into(),
                        name: tool.into(),
                        input: call_args.clone(),
                    }]),
                    meta: None,
                })
                .expect("call");
                log.append_message(MessageRecord {
                    message: Message {
                        role: Role::User,
                        content: vec![ContentBlock::ToolResult {
                            tool_use_id: "c1".into(),
                            content: "Card displayed to the user.".into(),
                            is_error: false,
                        }],
                    },
                    meta: None,
                })
                .expect("result");
                append_presentation_for_call(&mut log, tool, "c1", &call_args);
                let timeline = snapshot("conformance", &log);
                let found: Vec<_> = timeline
                    .items
                    .iter()
                    .filter_map(|i| match &i.content {
                        OutputContent::Structured { output } => Some(output.semantic_type.clone()),
                        _ => None,
                    })
                    .collect();
                // A card is answer content, never activity chatter: chat views
                // fold away progress/retry/information items, so a card
                // carrying one of those kinds is invisible to the user.
                if let Some(bad) = timeline.items.iter().find(|i| {
                    matches!(i.content, OutputContent::Structured { .. })
                        && i.kind != OutputKind::Card
                }) {
                    failures.push(format!(
                        "{semantic_type} via {tool}: card projected as {:?}, not Card",
                        bad.kind
                    ));
                }
                if found != [semantic_type.to_string()] {
                    failures.push(format!(
                        "{semantic_type} via {tool} padded={padded}: got {found:?}"
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} failures:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// The answer outline (docs/design/87-composed-answers.md §2): the
    /// plan's deliverables are the sections, a card is in the section its
    /// `answers` names, a card that names none is in no section, and the
    /// reply's text is in the one text section.
    /// A later `answer` in the same turn replaces the earlier one.
    #[test]
    fn a_later_answer_in_a_turn_replaces_the_earlier_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "answer-again");
        log.append_message(MessageRecord {
            message: Message::user_text("list what to pack"),
            meta: None,
        })
        .expect("directive");
        for (id, items) in [("a1", vec!["Charger"]), ("a2", vec!["Charger", "Hat"])] {
            let blocks = serde_json::json!({"blocks": [{"type": "list", "items": items}]});
            log.append_message(MessageRecord {
                message: Message::assistant(vec![ContentBlock::ToolUse {
                    id: id.into(),
                    name: "answer".into(),
                    input: blocks.clone(),
                }]),
                meta: None,
            })
            .expect("call");
            log.append_message(MessageRecord {
                message: Message {
                    role: Role::User,
                    content: vec![ContentBlock::ToolResult {
                        tool_use_id: id.into(),
                        content: "ok".into(),
                        is_error: false,
                    }],
                },
                meta: None,
            })
            .expect("result");
            append_presentation(
                &mut log,
                "emit_list_card",
                vak_session::types::PresentationSource::Answer {
                    tool_use_id: id.into(),
                    block: 0,
                },
                &serde_json::json!({"items": items}),
            );
        }
        let timeline = snapshot("answer-again", &log);
        let calls: Vec<Option<&str>> = timeline
            .items
            .iter()
            .filter(|item| item.kind == OutputKind::Card)
            .map(|item| {
                item.provenance
                    .as_ref()
                    .and_then(|provenance| provenance.tool_call_id.as_deref())
            })
            .collect();
        assert_eq!(calls, [Some("a2")]);
    }

    /// An `answer` call's cards are placed by what each block names, not
    /// by the call's own arguments or the blocks' order.
    #[test]
    fn an_answer_call_places_each_block_by_what_it_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "answer-outline");
        log.append_message(MessageRecord {
            message: Message::user_text("compare the two and list what to pack"),
            meta: None,
        })
        .expect("directive");
        let plan = serde_json::json!({"deliverables": [
            {"name": "comparison", "form": "card"},
            {"name": "packing list", "form": "card"}
        ]});
        let blocks = serde_json::json!({"blocks": [
            {"type": "text", "text": "Here it is."},
            {"type": "list", "items": ["Charger"], "deliverable": "packing list"},
            {"type": "list", "items": ["Train", "Car"], "deliverable": "Comparison"}
        ]});
        for (id, name, input) in [("p1", "plan", &plan), ("a1", "answer", &blocks)] {
            log.append_message(MessageRecord {
                message: Message::assistant(vec![ContentBlock::ToolUse {
                    id: id.into(),
                    name: name.into(),
                    input: input.clone(),
                }]),
                meta: None,
            })
            .expect("call");
            log.append_message(MessageRecord {
                message: Message {
                    role: Role::User,
                    content: vec![ContentBlock::ToolResult {
                        tool_use_id: id.into(),
                        content: "ok".into(),
                        is_error: false,
                    }],
                },
                meta: None,
            })
            .expect("result");
        }
        for block in [1_u32, 2] {
            append_presentation(
                &mut log,
                "emit_list_card",
                vak_session::types::PresentationSource::Answer {
                    tool_use_id: "a1".into(),
                    block,
                },
                &serde_json::json!({"items": blocks["blocks"][block as usize]["items"].clone()}),
            );
        }

        let timeline = snapshot("answer-outline", &log);
        let [outline] = timeline.answers.as_slice() else {
            panic!("one outline: {:?}", timeline.answers);
        };
        let cards: Vec<String> = timeline
            .items
            .iter()
            .filter(|item| item.kind == OutputKind::Card)
            .map(|item| item.id.clone())
            .collect();
        assert_eq!(cards.len(), 2, "{cards:?}");
        assert_eq!(outline.sections[0].items, [cards[1].clone()]);
        assert_eq!(outline.sections[1].items, [cards[0].clone()]);
    }

    #[test]
    fn actual_cards_fulfill_text_deliverables_without_positional_attribution() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "outline");
        log.append_message(MessageRecord {
            message: Message::user_text("compare the two and list what to pack"),
            meta: None,
        })
        .expect("directive");
        let call = |log: &mut SessionLog, id: &str, name: &str, input: serde_json::Value| {
            log.append_message(MessageRecord {
                message: Message::assistant(vec![ContentBlock::ToolUse {
                    id: id.into(),
                    name: name.into(),
                    input: input.clone(),
                }]),
                meta: None,
            })
            .expect("call");
            log.append_message(MessageRecord {
                message: Message {
                    role: Role::User,
                    content: vec![ContentBlock::ToolResult {
                        tool_use_id: id.into(),
                        content: "ok".into(),
                        is_error: false,
                    }],
                },
                meta: None,
            })
            .expect("result");
            if name != "plan" {
                append_presentation_for_call(log, name, id, &input);
            }
        };
        call(
            &mut log,
            "p1",
            "plan",
            serde_json::json!({"deliverables": [
                {"name": "comparison", "form": "text"},
                {"name": "packing list", "form": "text"},
                {"name": "advice", "form": "text"}
            ]}),
        );
        // Emitted in the other order from the plan, and one names nothing.
        call(
            &mut log,
            "c1",
            "emit_list_card",
            serde_json::json!({"items": ["Charger"], "deliverable": "Packing list"}),
        );
        call(
            &mut log,
            "c2",
            "emit_callout_card",
            serde_json::json!({"tone": "tip", "text": "Book early."}),
        );
        call(
            &mut log,
            "c3",
            "emit_list_card",
            serde_json::json!({"items": ["Train", "Car"], "deliverable": "comparison"}),
        );
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text("Take the train.")]),
            meta: None,
        })
        .expect("reply");

        let timeline = snapshot("outline", &log);
        let [outline] = timeline.answers.as_slice() else {
            panic!("one outline: {:?}", timeline.answers);
        };
        let item_of = |call: &str| {
            timeline
                .items
                .iter()
                .find(|item| {
                    item.provenance
                        .as_ref()
                        .and_then(|provenance| provenance.tool_call_id.as_deref())
                        == Some(call)
                        && item.kind == OutputKind::Card
                })
                .map(|item| item.id.clone())
                .unwrap_or_else(|| panic!("card for {call}"))
        };
        let names: Vec<&str> = outline.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["comparison", "packing list", "advice"],
            "plan order"
        );
        assert_eq!(outline.sections[0].form, "card");
        assert_eq!(outline.sections[1].form, "card");
        assert_eq!(outline.sections[2].form, "text");
        assert_eq!(outline.sections[0].items, [item_of("c3")]);
        assert_eq!(outline.sections[1].items, [item_of("c1")]);
        assert_eq!(
            outline.sections[2].items.len(),
            1,
            "the reply's text: {:?}",
            timeline
                .items
                .iter()
                .map(|item| (item.kind, item.role, item.turn_id.clone()))
                .collect::<Vec<_>>()
        );
        let placed: Vec<&String> = outline.sections.iter().flat_map(|s| &s.items).collect();
        assert!(
            !placed.contains(&&item_of("c2")),
            "a card that names no deliverable is in no section"
        );
    }

    /// A card sent with `replaces` takes the place of that card in the
    /// answer (the ledger keeps both), and a card sent with
    /// `beside_previous` is listed for the client to place beside the one
    /// before it. Both are the model's own typed choices.
    #[test]
    fn a_card_replaces_the_card_it_names_and_placement_is_the_models() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "replaces");
        log.append_message(MessageRecord {
            message: Message::user_text("a five k plan"),
            meta: None,
        })
        .expect("directive");
        let call = |log: &mut SessionLog, id: &str, name: &str, input: serde_json::Value| {
            log.append_message(MessageRecord {
                message: Message::assistant(vec![ContentBlock::ToolUse {
                    id: id.into(),
                    name: name.into(),
                    input: input.clone(),
                }]),
                meta: None,
            })
            .expect("call");
            log.append_message(MessageRecord {
                message: Message {
                    role: Role::User,
                    content: vec![ContentBlock::ToolResult {
                        tool_use_id: id.into(),
                        content: "ok".into(),
                        is_error: false,
                    }],
                },
                meta: None,
            })
            .expect("result");
            append_presentation_for_call(log, name, id, &input);
            log.presentations().last().expect("recorded").0.clone()
        };
        call(
            &mut log,
            "c0",
            "emit_callout_card",
            serde_json::json!({"tone":"note", "text":"Earlier independent block."}),
        );
        let first = call(
            &mut log,
            "c1",
            "emit_list_card",
            serde_json::json!({"items": ["Week 1: walk"]}),
        );
        call(
            &mut log,
            "c2",
            "emit_callout_card",
            serde_json::json!({"tone": "tip", "text": "Rest between runs.", "beside_previous": true}),
        );
        let paired_timeline = snapshot("replaces", &log);
        let pairs = &paired_timeline.answers[0].beside;
        assert_eq!(pairs.len(), 1);
        assert!(pairs[0][0].starts_with("c1-") && pairs[0][1].starts_with("c2-"));
        // Feed the actual accepted records through the live frame projector.
        // Endpoint identities and placement must match the settled snapshot.
        let mut live = OutputTimeline::empty("replaces");
        for (sequence, (presentation_id, record)) in log.presentations().into_iter().enumerate() {
            let vak_session::types::PresentationSource::ToolCall { tool_use_id } = &record.source
            else {
                continue;
            };
            super::project_frame(
                &mut live,
                crate::events::SeqEvent {
                    seq: sequence as u64 + 1,
                    event: vak_agent::AgentEvent::CardRecorded {
                        tool_use_id: tool_use_id.clone(),
                        presentation_id,
                        record: Box::new(record.clone()),
                    },
                },
            );
        }
        let settled_cards: Vec<_> = paired_timeline
            .items
            .iter()
            .filter(|item| item.kind == OutputKind::Card)
            .map(|item| (&item.id, &item.placement))
            .collect();
        let live_cards: Vec<_> = live
            .items
            .iter()
            .filter(|item| item.kind == OutputKind::Card)
            .map(|item| (&item.id, &item.placement))
            .collect();
        assert_eq!(
            live_cards, settled_cards,
            "accepted identities and anchors survive settlement"
        );
        let mut settled_then_late = paired_timeline.clone();
        let before_late = settled_then_late.items.clone();
        for item in live.items.clone() {
            super::apply_stream_event(
                &mut settled_then_late,
                vak_delivery::presentation::OutputStreamEvent::ItemStarted { item },
            );
        }
        assert_eq!(
            settled_then_late.items, before_late,
            "late live events never overwrite settled card identity or turn attribution"
        );

        let third = call(
            &mut log,
            "c3",
            "emit_list_card",
            serde_json::json!({"items": ["Week 1: run one minute"], "replaces": first}),
        );
        let (presentation_id, record) = log
            .presentations()
            .last()
            .map(|(id, record)| (id.clone(), (*record).clone()))
            .unwrap();
        super::project_frame(
            &mut live,
            crate::events::SeqEvent {
                seq: 10,
                event: vak_agent::AgentEvent::CardRecorded {
                    tool_use_id: "c3".into(),
                    presentation_id,
                    record: Box::new(record),
                },
            },
        );
        assert!(
            !live.items.iter().any(|item| item.id.starts_with("c1-")),
            "live replacement hides its exact target"
        );
        assert_eq!(
            live.items
                .iter()
                .find(|item| item.id.starts_with("c2-"))
                .unwrap()
                .placement
                .beside
                .as_deref(),
            Some(first.as_str()),
            "live anchor never rebinds to c0"
        );
        let timeline = snapshot("replaces", &log);
        let cards: Vec<String> = timeline
            .items
            .iter()
            .filter(|item| item.kind == OutputKind::Card)
            .filter_map(|item| item.provenance.as_ref()?.tool_call_id.clone())
            .collect();
        assert_eq!(cards, ["c0", "c2", "c3"], "c3 takes c1's place");
        assert!(
            timeline.answers.is_empty(),
            "a removed placement anchor never rebinds to another card"
        );
        let mut covered = timeline.clone();
        covered.covered_sequence = Some(10);
        let before = covered.items.clone();
        let (old_id, old) = log
            .presentations()
            .into_iter()
            .find(|(id, _)| id == &first)
            .unwrap();
        assert!(
            super::project_frame(
                &mut covered,
                crate::events::SeqEvent {
                    seq: 2,
                    event: vak_agent::AgentEvent::CardRecorded {
                        tool_use_id: "c1".into(),
                        presentation_id: old_id,
                        record: Box::new(old.clone())
                    }
                }
            )
            .is_none()
        );
        assert_eq!(
            covered.items, before,
            "covered late events cannot resurrect a retired anchor"
        );
        let mut next = old.clone();
        next.turn_id = "next-directive".into();
        super::project_frame(
            &mut covered,
            crate::events::SeqEvent {
                seq: 11,
                event: vak_agent::AgentEvent::CardRecorded {
                    tool_use_id: "next-card".into(),
                    presentation_id: "next-pres".into(),
                    record: Box::new(next),
                },
            },
        );
        assert!(
            covered
                .items
                .iter()
                .any(|item| item.id == "next-card-structured-0"),
            "new turn events remain admitted after the settlement watermark"
        );
        // A replacement may restore an earlier payload. Deduplication uses
        // active cards, so the retired first payload cannot hide this one.
        call(
            &mut log,
            "c4",
            "emit_list_card",
            serde_json::json!({"items": ["Week 1: walk"], "replaces": third}),
        );
        let timeline = snapshot("replaces", &log);
        let cards: Vec<_> = timeline
            .items
            .iter()
            .filter(|item| item.kind == OutputKind::Card)
            .filter_map(|item| item.provenance.as_ref()?.tool_call_id.as_deref())
            .collect();
        assert_eq!(cards, ["c0", "c2", "c4"]);
        assert_eq!(log.presentations().len(), 5, "history remains intact");
    }

    fn channel_log(dir: &tempfile::TempDir, name: &str) -> SessionLog {
        SessionLog::create(
            dir.path().join(format!("{name}.jsonl")),
            SessionHeader {
                space: None,
                run: None,
                cause: None,
                agent: None,
                session_id: name.into(),
                created_at: chrono::Utc::now(),
                cwd: PathBuf::from("/tmp/project"),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
                conversation: None,
                contract: FrozenContract {
                    app_version: "test".into(),
                    provider: "test".into(),
                    model: "test".into(),
                    route_ladder: Vec::new(),
                    route_objective: String::new(),
                    route_annotations: Vec::new(),
                    system_prompt: String::new(),
                    permission_mode: "read-only".into(),
                    capabilities: Vec::new(),
                    prompt_layers: Vec::new(),
                },
            },
        )
        .expect("create")
    }

    /// A turn that answered with a card and no words has a result its draft
    /// is reviewed under: the card. Found live 2026-10-12, when such a
    /// turn's draft could not be opened for Review.
    #[test]
    fn a_turn_answered_with_a_card_alone_has_a_result_for_review() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "card-only");
        log.append_message(MessageRecord {
            message: Message::user_text("Chart the week and save it."),
            meta: None,
        })
        .expect("request");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::ToolUse {
                id: "w1".into(),
                name: "write".into(),
                input: serde_json::json!({"path": "week.md", "content": "Mon 1"}),
            }]),
            meta: None,
        })
        .expect("call");
        log.append_message(MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "w1".into(),
                    content: "Wrote week.md".into(),
                    is_error: false,
                }],
            },
            meta: None,
        })
        .expect("result");
        let timeline = snapshot("card-only", &log);
        assert_eq!(
            super::result_of_execution(&timeline, "w1"),
            Some(Err(())),
            "nothing answered yet"
        );
        assert_eq!(super::result_of_execution(&timeline, "other"), None);

        append_card_call(&mut log, "c1", "the week");
        let timeline = snapshot("card-only", &log);
        let card = timeline
            .items
            .iter()
            .find(|item| item.kind == OutputKind::Card)
            .expect("the card is projected");
        let (turn, result) = super::result_of_execution(&timeline, "w1")
            .expect("the call is in the timeline")
            .expect("the card is the result");
        assert_eq!(turn, card.turn_id);
        assert_eq!(result, card.id);
    }

    fn append_card_call(log: &mut SessionLog, id: &str, summary: &str) {
        let input = serde_json::json!({"semantic_type":"chart","chart_type":"line","data":[{"series":"s","x":"Mon","y":1}],"accessible_summary":summary});
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::ToolUse {
                id: id.into(),
                name: "emit_chart_card".into(),
                input: input.clone(),
            }]),
            meta: None,
        })
        .expect("call");
        log.append_message(MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: id.into(),
                    content: "Card displayed to the user (chart).".into(),
                    is_error: false,
                }],
            },
            meta: None,
        })
        .expect("result");
        append_presentation_for_call(log, "emit_chart_card", id, &input);
    }

    /// Channel fallbacks include the card projection and every assistant text
    /// block from the turn; text prefixes have no control meaning.
    #[test]
    fn a_channel_gets_the_cards_of_the_run_ahead_of_the_narration() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "ch1");
        log.append_message(MessageRecord {
            message: Message::user_text("chart it"),
            meta: None,
        })
        .expect("u");
        append_card_call(&mut log, "c1", "sales rise steadily");
        let text = super::text_with_run_cards(&log, "The chart is shown above.".into());
        assert!(
            text.contains("sales rise steadily"),
            "card content must reach the channel: {text}"
        );
        assert!(
            !text.contains("```json"),
            "channel fallback must not leak the audit appendix: {text}"
        );
        let cards = super::run_cards(&log);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].semantic_type, "chart");
        assert!(text.contains("The chart is shown above."), "{text}");
        let with_note = super::text_with_run_cards(&log, "Note: Check the holiday dip.".into());
        assert!(with_note.contains("sales rise steadily"), "{with_note}");
        assert!(with_note.ends_with("Check the holiday dip."), "{with_note}");
    }

    #[test]
    fn card_result_projects_natural_followup_without_rewriting_its_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "card-note");
        log.append_message(MessageRecord {
            message: Message::user_text("chart it"),
            meta: None,
        })
        .expect("user");
        append_card_call(&mut log, "c1", "sales rise steadily");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text(
                "Note: The holiday dip needs review.",
            )]),
            meta: None,
        })
        .expect("answer");
        let timeline = super::snapshot("card-note", &log);
        let note = timeline.items.iter().find_map(|item| match &item.content {
            OutputContent::Document { document } => document.metadata.get("card_followup"),
            _ => None,
        });
        assert_eq!(
            note.map(String::as_str),
            Some("Note: The holiday dip needs review.")
        );
    }

    #[test]
    fn context_before_the_first_card_remains_visible() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "card-leading-context");
        log.append_message(MessageRecord {
            message: Message::user_text("make a chart"),
            meta: None,
        })
        .expect("user");
        let context = "Requests rose through Wednesday, then eased.";
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text(context)]),
            meta: None,
        })
        .expect("context");
        append_card_call(&mut log, "c1", "requests by day");

        let timeline = super::snapshot("card-leading-context", &log);
        assert!(timeline.items.iter().any(|item| match &item.content {
            OutputContent::Document { document } => document.source_markdown == context,
            OutputContent::Outcome {
                document: Some(document),
                ..
            } => document.source_markdown == context,
            _ => false,
        }));
        assert!(
            timeline
                .items
                .iter()
                .any(|item| item.kind == OutputKind::Card)
        );
    }

    #[test]
    fn prose_and_distinct_cards_keep_their_projected_source_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "interleaved-card-context");
        log.append_message(MessageRecord {
            message: Message::user_text("compare both results"),
            meta: None,
        })
        .expect("user");
        for (index, text) in [
            "Leading context survives.",
            "Note: the second result uses a different scope.",
            "The closing explanation survives too.",
        ]
        .into_iter()
        .enumerate()
        {
            log.append_message(MessageRecord {
                message: Message::assistant(vec![ContentBlock::text(text)]),
                meta: None,
            })
            .expect("prose");
            if index < 2 {
                append_card_call(&mut log, &format!("c{index}"), &format!("result {index}"));
            }
        }
        let timeline = super::snapshot("interleaved-card-context", &log);
        let order: Vec<String> = timeline
            .items
            .iter()
            .filter_map(|item| {
                if item.kind == OutputKind::Card {
                    return Some("card".into());
                }
                if item.role != vak_delivery::OutputRole::Assistant {
                    return None;
                }
                match &item.content {
                    OutputContent::Document { document } => Some(document.source_markdown.clone()),
                    OutputContent::Outcome {
                        document: Some(document),
                        ..
                    } => Some(document.source_markdown.clone()),
                    _ => None,
                }
            })
            .collect();
        assert_eq!(
            order,
            vec![
                "Leading context survives.",
                "card",
                "Note: the second result uses a different scope.",
                "card",
                "The closing explanation survives too.",
            ]
        );
    }

    #[test]
    fn card_result_projects_completion_text_as_written() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "card-completion-narration");
        log.append_message(MessageRecord {
            message: Message::user_text("make a schedule and budget"),
            meta: None,
        })
        .expect("user");
        append_card_call(&mut log, "c1", "schedule and budget details");
        let narration = "The requested plan, including the 90-minute schedule, essential checklist, and budget allocation, is complete. No further information is required.";
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text(narration)]),
            meta: None,
        })
        .expect("answer");

        let timeline = super::snapshot("card-completion-narration", &log);
        assert!(
            timeline
                .items
                .iter()
                .any(|item| item.kind == OutputKind::Card)
        );
        assert!(
            timeline.items.iter().any(|item| match &item.content {
                OutputContent::Document { document } => document
                    .metadata
                    .get("card_followup")
                    .is_some_and(|text| text == narration),
                OutputContent::Outcome {
                    document: Some(document),
                    ..
                } => document
                    .metadata
                    .get("card_followup")
                    .is_some_and(|text| text == narration),
                _ => false,
            }),
            "follow-up is projected as written, without phrase classification"
        );
        assert!(
            log.active_entries_rev().any(|entry| match &entry.payload {
                vak_session::EntryPayload::Message(record) =>
                    record.message.content.iter().any(|block| {
                        matches!(block, ContentBlock::Text { text } if text == narration)
                    }),
                _ => false,
            }),
            "original assistant text remains available in the ledger"
        );
    }

    #[test]
    fn card_result_does_not_give_note_prefix_control_meaning() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "card-marked-completion-boilerplate");
        log.append_message(MessageRecord {
            message: Message::user_text("show me the plan in cards"),
            meta: None,
        })
        .expect("user");
        append_card_call(&mut log, "c1", "Volunteer Orientation Plan Summary");
        let narration = "Note: All distinct parts of the request have been addressed. No further text is needed.";
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text(narration)]),
            meta: None,
        })
        .expect("answer");

        let timeline = super::snapshot("card-marked-completion-boilerplate", &log);
        assert!(
            timeline
                .items
                .iter()
                .any(|item| item.kind == OutputKind::Card)
        );
        assert!(
            timeline.items.iter().any(|item| match &item.content {
                OutputContent::Document { document } => document
                    .metadata
                    .get("card_followup")
                    .is_some_and(|text| text == narration),
                OutputContent::Outcome {
                    document: Some(document),
                    ..
                } => document
                    .metadata
                    .get("card_followup")
                    .is_some_and(|text| text == narration),
                _ => false,
            }),
            "Note: remains ordinary text and is projected in full"
        );
        assert!(
            log.active_entries_rev().any(|entry| match &entry.payload {
                vak_session::EntryPayload::Message(record) =>
                    record.message.content.iter().any(|block| {
                        matches!(block, ContentBlock::Text { text } if text == narration)
                    }),
                _ => false,
            }),
            "original assistant text remains available in the ledger"
        );
    }

    #[test]
    fn card_result_preserves_followup_even_when_it_repeats_card_facts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "card-duplicate-note");
        log.append_message(MessageRecord {
            message: Message::user_text("when does the book club start and what is the budget?"),
            meta: None,
        })
        .expect("user");
        append_card_call(
            &mut log,
            "c1",
            "Book Club Plan — Start Time 6 pm; Total Budget $60",
        );
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text(
                "Note: The book club plan specified a start time of 6 pm and a total budget of $60.",
            )]),
            meta: None,
        })
        .expect("answer");

        let timeline = super::snapshot("card-duplicate-note", &log);
        assert!(
            timeline.items.iter().any(|item| match &item.content {
                OutputContent::Document { document } => document
                    .metadata
                    .get("card_followup")
                    .is_some_and(|text| text.contains("start time of 6 pm")),
                OutputContent::Outcome {
                    document: Some(document),
                    ..
                } => document
                    .metadata
                    .get("card_followup")
                    .is_some_and(|text| text.contains("start time of 6 pm")),
                _ => false,
            }),
            "projection preserves source text and leaves deduplication to the model"
        );
        assert!(
            timeline
                .items
                .iter()
                .any(|item| item.kind == OutputKind::Card)
        );
    }

    fn link_cards(timeline: &vak_delivery::OutputTimeline) -> usize {
        timeline
            .items
            .iter()
            .filter(|item| match &item.content {
                OutputContent::Structured { output } => output.semantic_type == "link.preview",
                _ => false,
            })
            .count()
    }

    #[test]
    fn a_url_in_plain_text_is_not_turned_into_a_card() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "plain-url");
        log.append_message(MessageRecord {
            message: Message::user_text("what is on example.com"),
            meta: None,
        })
        .expect("user");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text(
                "I could not fetch https://example.com, the network is closed.",
            )]),
            meta: None,
        })
        .expect("answer");
        assert_eq!(link_cards(&super::snapshot("plain-url", &log)), 0);
    }

    #[test]
    fn a_run_with_no_cards_is_delivered_unchanged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "ch2");
        log.append_message(MessageRecord {
            message: Message::user_text("hi"),
            meta: None,
        })
        .expect("u");
        assert_eq!(super::text_with_run_cards(&log, "hello".into()), "hello");
    }

    #[test]
    fn a_card_from_an_earlier_turn_does_not_suppress_channel_narration() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "channel-history");
        log.append_message(MessageRecord {
            message: Message::user_text("first, chart the sales"),
            meta: None,
        })
        .expect("first user");
        append_card_call(&mut log, "c1", "sales rise steadily");
        log.append_message(MessageRecord {
            message: Message::user_text("second, explain the result"),
            meta: None,
        })
        .expect("second user");

        let narration = "The sales increased throughout the period.";
        let (text, metadata) = super::channel_answer(&log, narration.into());
        assert_eq!(text, narration);
        assert!(
            metadata.is_none(),
            "an earlier turn cannot lend its outcome to this reply"
        );
    }

    #[test]
    fn only_the_latest_turns_cards_are_delivered() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "ch3");
        log.append_message(MessageRecord {
            message: Message::user_text("first"),
            meta: None,
        })
        .expect("u1");
        append_card_call(&mut log, "c1", "OLD-TURN-CARD");
        log.append_message(MessageRecord {
            message: Message::user_text("second"),
            meta: None,
        })
        .expect("u2");
        append_card_call(&mut log, "c2", "NEW-TURN-CARD");
        let text = super::text_with_run_cards(&log, "done".into());
        assert!(
            text.contains("NEW-TURN-CARD") && !text.contains("OLD-TURN-CARD"),
            "{text}"
        );
    }

    #[test]
    /// A runtime nudge is not a new turn: a card sent after it belongs to
    /// the same answer, beside the earlier one. Only the model's own
    /// `replaces` takes a card's place (docs/design/87 "Placement and
    /// revision").
    fn a_card_after_a_nudge_is_in_the_same_answer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "ch4");
        log.append_message(MessageRecord {
            message: Message::user_text("chart it"),
            meta: None,
        })
        .expect("u");
        append_card_call(&mut log, "c1", "FIRST-ATTEMPT");
        log.append_message(MessageRecord::control(
            vak_intent::control::ControlKind::FenceCheck,
            "[fence-check]: resend",
        ))
        .expect("nudge");
        append_card_call(&mut log, "c2", "RETRY");
        let text = super::text_with_run_cards(&log, String::new());
        let first = text.find("FIRST-ATTEMPT").expect("first card");
        let retry = text.find("RETRY").expect("second card");
        assert!(first < retry, "{text}");
    }

    /// Real ledger (gemma, "weather in noida"): a prose draft, a presentation
    /// nudge, the same card called three times, then the narration. The user
    /// sees one card and one answer, in that order.
    #[test]
    fn identical_repeated_card_calls_and_a_rejected_draft_project_to_one_card_one_answer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "repeat");
        log.append_message(MessageRecord {
            message: Message::user_text("weather in noida"),
            meta: None,
        })
        .expect("u");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text("DRAFT 28C")]),
            meta: None,
        })
        .expect("draft");
        log.append_message(MessageRecord::control(
            vak_intent::control::ControlKind::PresentationCheck,
            "[presentation-check]: use a card",
        ))
        .expect("nudge");
        for id in ["c1", "c2", "c3"] {
            append_card_call(&mut log, id, "28C");
        }
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text("FINAL 28C")]),
            meta: None,
        })
        .expect("final");
        let timeline = snapshot("repeat", &log);
        let cards = timeline
            .items
            .iter()
            .filter(|i| i.kind == OutputKind::Card)
            .count();
        let answers: Vec<&str> = timeline
            .items
            .iter()
            .filter(|i| i.role == vak_delivery::OutputRole::Assistant)
            .map(|i| i.fallback_text.as_str())
            .collect();
        assert_eq!(
            cards,
            1,
            "{:?}",
            timeline.items.iter().map(|i| &i.id).collect::<Vec<_>>()
        );
        assert_eq!(answers.len(), 1, "{answers:?}");
        assert!(answers[0].contains("FINAL"), "{answers:?}");
        let kinds: Vec<_> = timeline.items.iter().map(|i| i.kind).collect();
        let card_at = kinds
            .iter()
            .position(|k| *k == OutputKind::Card)
            .expect("a card");
        let answer_at = timeline
            .items
            .iter()
            .position(|i| i.role == vak_delivery::OutputRole::Assistant)
            .expect("an answer");
        assert!(card_at < answer_at, "the card precedes its narration");
    }

    /// Text written in reply to a plan check that followed an answer is
    /// addressed to the runtime: the person keeps the answer, and a card the
    /// reply adds is still shown.
    #[test]
    fn a_reply_to_a_plan_check_after_an_answer_is_not_projected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "plan-reply");
        log.append_message(MessageRecord {
            message: Message::user_text("plan a trip"),
            meta: None,
        })
        .expect("u");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text("THE-ANSWER itinerary")]),
            meta: None,
        })
        .expect("answer");
        log.append_message(MessageRecord::control(
            vak_intent::control::ControlKind::PlanCheck,
            "[plan-check]: the plan's budget card is missing",
        ))
        .expect("check");
        append_card_call(&mut log, "c1", "Budget 500");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text("STATUS-REPLY all cards shown")]),
            meta: None,
        })
        .expect("reply");
        let timeline = snapshot("plan-reply", &log);
        let assistant_text: Vec<String> = timeline
            .items
            .iter()
            .filter(|i| i.role == vak_delivery::OutputRole::Assistant)
            .map(|i| i.fallback_text.clone())
            .collect();
        assert!(
            assistant_text.iter().any(|t| t.contains("THE-ANSWER")),
            "{assistant_text:?}"
        );
        assert!(
            !assistant_text.iter().any(|t| t.contains("STATUS-REPLY")),
            "{assistant_text:?}"
        );
        assert!(
            timeline
                .items
                .iter()
                .any(|i| i.kind == OutputKind::Card && i.fallback_text.contains("Budget 500")),
            "{:?}",
            timeline.items
        );
    }

    /// A draft the runtime sent back for a redo is internal: the user sees the
    /// redone answer, never the rejected attempt as well.
    #[test]
    fn a_rejected_draft_is_never_projected_but_the_redo_is() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "draft");
        log.append_message(MessageRecord {
            message: Message::user_text("weather in delhi"),
            meta: None,
        })
        .expect("u");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text(
                "DRAFT-PROSE answer with a broken fence",
            )]),
            meta: None,
        })
        .expect("draft");
        log.append_message(MessageRecord::control(
            vak_intent::control::ControlKind::PresentationCheck,
            "[presentation-check]: use a card",
        ))
        .expect("nudge");
        append_card_call(&mut log, "c1", "29C and sunny");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text("FINAL-NARRATION")]),
            meta: None,
        })
        .expect("final");
        let timeline = snapshot("draft", &log);
        let assistant_text: Vec<String> = timeline
            .items
            .iter()
            .filter(|i| i.role == vak_delivery::OutputRole::Assistant)
            .map(|i| i.fallback_text.clone())
            .collect();
        assert!(
            !assistant_text.iter().any(|t| t.contains("DRAFT-PROSE")),
            "{assistant_text:?}"
        );
        assert!(
            assistant_text.iter().any(|t| t.contains("FINAL-NARRATION")),
            "{assistant_text:?}"
        );
        assert!(
            timeline
                .items
                .iter()
                .any(|i| matches!(i.content, OutputContent::Structured { .. }))
        );
    }

    /// Only a redo-nudge rejects a draft. A stop hook or guard asks the model
    /// to keep working, so the text before it is real interim narration.
    #[test]
    fn a_continue_nudge_does_not_reject_the_text_before_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "cont");
        log.append_message(MessageRecord {
            message: Message::user_text("do the thing"),
            meta: None,
        })
        .expect("u");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text("INTERIM-PROGRESS-NOTE")]),
            meta: None,
        })
        .expect("interim");
        log.append_message(MessageRecord::control(
            vak_intent::control::ControlKind::StopGuard,
            vak_control::render(
                &vak_control::Finding::StopGuard {
                    reason: "not done yet".into(),
                },
                "[stop-guard]",
            ),
        ))
        .expect("guard");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text("ALL-DONE")]),
            meta: None,
        })
        .expect("final");
        let timeline = snapshot("cont", &log);
        let texts: Vec<String> = timeline
            .items
            .iter()
            .filter(|i| i.role == vak_delivery::OutputRole::Assistant)
            .map(|i| i.fallback_text.clone())
            .collect();
        assert!(
            texts.iter().any(|t| t.contains("INTERIM-PROGRESS-NOTE")),
            "{texts:?}"
        );
        assert!(texts.iter().any(|t| t.contains("ALL-DONE")), "{texts:?}");
    }

    /// The transcript sent to clients carries only what a person can see:
    /// nudges and derived context blocks are not sent, and every message
    /// arrives with the ledger entry id the chat pairs turns by.
    #[test]
    fn the_client_transcript_omits_runtime_traffic_and_carries_entry_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "wire");
        log.append_message(MessageRecord {
            message: Message::user_text("what is the weather"),
            meta: None,
        })
        .expect("u");
        // A text-only draft the runtime itself rejected: the very next
        // entry is a control nudge that asks for a redo. Neither the draft
        // nor the nudge belongs on the wire (docs/audits Finding 3).
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text("sunny")]),
            meta: None,
        })
        .expect("draft");
        log.append_message(MessageRecord::control(
            vak_intent::control::ControlKind::GroundingCheck,
            "[grounding-check]: cite it",
        ))
        .expect("nudge");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text("Sunny, source: NWS.")]),
            meta: None,
        })
        .expect("final answer");
        let json = crate::transcript_json(&log);
        let messages = json["messages"].as_array().expect("messages");
        let entries = json["entries"].as_array().expect("entries");
        assert_eq!(
            messages.len(),
            2,
            "neither the nudge nor the rejected draft is on the wire: {messages:?}"
        );
        assert_eq!(
            messages[1]["content"][0]["text"].as_str(),
            Some("Sunny, source: NWS."),
            "the redo's real answer is on the wire, not the discarded draft"
        );
        assert_eq!(
            entries.len(),
            messages.len(),
            "entries run parallel to messages"
        );
        assert!(
            entries
                .iter()
                .all(|e| e["entry_id"].as_str().is_some_and(|id| !id.is_empty()))
        );
        assert!(
            json.get("contract").is_none(),
            "the frozen system prompt is not sent to the client"
        );
        assert!(
            json["count"].as_u64().expect("count") >= 4,
            "count stays the model-visible total, draft and nudge included"
        );
    }

    #[test]
    fn conversation_messages_drops_the_rejected_draft_but_keeps_the_redo() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "wire-md");
        log.append_message(MessageRecord {
            message: Message::user_text("what is the weather"),
            meta: None,
        })
        .expect("u");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text("sunny")]),
            meta: None,
        })
        .expect("draft");
        log.append_message(MessageRecord::control(
            vak_intent::control::ControlKind::GroundingCheck,
            "[grounding-check]: cite it",
        ))
        .expect("nudge");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text("Sunny, source: NWS.")]),
            meta: None,
        })
        .expect("final answer");
        let messages = crate::conversation_messages(&log);
        let texts: Vec<String> = messages
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["what is the weather", "Sunny, source: NWS."]);
    }
}

#[cfg(test)]
mod outcome_field_tests {
    use super::*;

    #[test]
    fn card_only_answers_carry_the_same_evidence_verdict_and_users_never_do() {
        let mut timeline: OutputTimeline = serde_json::from_value(serde_json::json!({
            "schema_version": 2, "session_id": "s", "diagnostics": [], "items": [
                {"id":"user", "turn_id":"turn-1", "role":"user", "kind":"message", "status":"succeeded", "timestamp":"now", "provenance":{"entry_id":"directive"}, "content":{"type":"progress","label":"question"}, "actions":[], "fallback_text":"question"},
                {"id":"card", "turn_id":"turn-1", "role":"tool", "kind":"card", "status":"succeeded", "timestamp":"now", "content":{"type":"progress","label":"answer"}, "actions":[], "fallback_text":"answer"}
            ]
        })).unwrap();
        let evaluations = HashMap::from([(
            "directive".into(),
            TurnEvaluation {
                status: "produced".into(),
                evaluation: "[]".into(),
                completion: Some("unknown".into()),
                receipts: vec!["source-call".into()],
                requirement_ids: vec!["question-1".into()],
            },
        )]);
        apply_turn_evaluations(
            &mut timeline,
            &evaluations,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        );
        assert!(timeline.items[0].outcome.is_none());
        let result = timeline.items[1].outcome.as_ref().unwrap();
        assert_eq!(result.completion.as_deref(), Some("unknown"));
        assert_eq!(result.requirement_ids, vec!["question-1"]);
        assert_eq!(result.evidence_receipt_ids, vec!["source-call"]);
        assert_eq!(timeline.items[1].status, OutputStatus::Partial);
    }

    #[test]
    fn authored_questions_cannot_change_completion_or_receipt_links() {
        let evaluation = serde_json::json!([{"requirement_id":"plan-evidence-1-1","status":"unknown","reason":"Does A | B hold? Neither is established | complete"}]).to_string();
        let activity = vak_session::ActivityRecord {
            activity_id: "evaluation".into(),
            kind: ActivityKind::Diagnostic,
            status: vak_session::ActivityStatus::Succeeded,
            label: "Outcome evaluation".into(),
            detail: None,
            data: BTreeMap::from([
                ("status".into(), "produced".into()),
                ("evaluation".into(), evaluation.clone()),
                ("evidence_receipts".into(), "call-a,call-b".into()),
                ("completion".into(), "unknown".into()),
            ]),
        };
        let projected = TurnEvaluation::from_activity(&activity);
        let result = result_outcome(
            "result",
            OutputStatus::Partial,
            None,
            Some(&projected),
            None,
            None,
        );
        assert_eq!(result.completion.as_deref(), Some("unknown"));
        assert_eq!(result.requirement_ids, vec!["plan-evidence-1-1"]);
        assert_eq!(result.evidence_receipt_ids, vec!["call-a", "call-b"]);
        assert_eq!(projected.evaluation, evaluation);
        let mut corrupt = activity;
        corrupt.data.insert("evaluation".into(), "not JSON".into());
        corrupt.data.insert("completion".into(), "complete".into());
        assert_eq!(
            TurnEvaluation::from_activity(&corrupt)
                .completion
                .as_deref(),
            Some("unknown")
        );
    }
}
