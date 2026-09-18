use std::collections::{BTreeMap, HashMap};

use vak_agent::AgentEvent;
use vak_delivery::{
    ArtifactRef, DeliveryAction, OutputContent, OutputItem, OutputKind, OutputProvenance,
    OutputRole, OutputStatus, OutputStreamEvent, OutputTimeline, PresentationPlanner,
    ResultOutcome, SignalContext, built_in_adapters, compile_markdown, link_previews_from_text,
    signals_from_context, structured_markdown, structured_outputs_from_text,
    structured_outputs_from_tool_result_with,
};

fn status_for_completion(completion: Option<&str>) -> OutputStatus {
    if completion.is_none() || completion.is_some_and(|value| value.trim() == "complete") {
        OutputStatus::Succeeded
    } else {
        OutputStatus::Partial
    }
}

fn result_outcome(
    result_id: impl Into<String>,
    status: OutputStatus,
    admitted: Option<&vak_intent::OutcomeSpec>,
    evaluation: Option<&str>,
    evidence_state: Option<&String>,
    human_review: Option<&String>,
) -> Option<ResultOutcome> {
    let completion = evaluation
        .and_then(|value| value.split('|').nth(3))
        .map(str::to_owned);
    if admitted.is_none()
        && completion.is_none()
        && evidence_state.is_none()
        && human_review.is_none()
    {
        return None;
    }
    Some(ResultOutcome {
        result_id: result_id.into(),
        status,
        completion,
        evidence_state: evidence_state.cloned(),
        requirement_ids: admitted
            .map(|outcome| {
                outcome
                    .requirements
                    .iter()
                    .map(|item| item.id.clone())
                    .collect()
            })
            .unwrap_or_default(),
        evidence_receipt_ids: evaluation
            .and_then(|value| value.split('|').nth(2))
            .map(|value| {
                value
                    .split(',')
                    .filter(|item| !item.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        evidence: Vec::new(),
        human_review: human_review.cloned(),
    })
}
use vak_llm::{ContentBlock, Role};
use vak_session::{ActivityKind, ActivityStatus, EntryPayload, SessionLog};

/// One tool call as the timeline needs it: name, input, result text, and
/// whether the result was an error. Named because the inline tuple was wide
/// enough that a reader had to count commas to find the error flag.
type TurnTool = (String, String, serde_json::Value, Option<String>, bool);

pub(crate) fn snapshot(session_id: &str, session: &SessionLog) -> OutputTimeline {
    let builtin = PresentationPlanner {
        skills: vak_delivery::built_in_skill_registry(),
        recipes: vak_delivery::built_in_recipes(),
    };
    snapshot_inner(session_id, session, &builtin, None)
}

/// Like [`snapshot`] but uses a plugin-merged `PresentationPlanner` so that
/// domain-specific recipes and semantic types are recognized during
/// live projection. Callers with Core access should use this for SSE and
/// live session handles; historical views without Core can use `snapshot`.
pub(crate) fn snapshot_with_planner(
    session_id: &str,
    session: &SessionLog,
    planner: &PresentationPlanner,
) -> OutputTimeline {
    snapshot_inner(session_id, session, planner, None)
}

/// Live projection variant with the persisted adaptive library. The legacy
/// planner remains authoritative for validation; the library only contributes
/// an optional, auditable selection reference to the same document.
pub(crate) fn snapshot_with_planner_and_library(
    session_id: &str,
    session: &SessionLog,
    planner: &PresentationPlanner,
    library: &vak_presentation::PresentationLibrary,
) -> OutputTimeline {
    snapshot_inner(session_id, session, planner, Some(library))
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
    let path = home
        .join("sandbox")
        .join("executions")
        .join(format!("{session_id}.jsonl"));
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    for line in text.lines() {
        let Ok(event) = serde_json::from_str::<vak_tools::SandboxEvent>(line) else {
            continue;
        };
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
        timeline.items.push(OutputItem {
            id,
            timestamp: chrono::Utc::now().to_rfc3339(),
            turn_id: format!("sandbox-{execution_id}"),
            role: OutputRole::Tool,
            kind: OutputKind::Artifact,
            status: OutputStatus::Succeeded,
            outcome: None,
            content: OutputContent::Artifact {
                artifact: ArtifactRef {
                    name,
                    path: Some(path.clone()),
                    media_type: Some(mime_type),
                    description: Some(format!(
                        "Generated by sandbox execution ({size_bytes} bytes)"
                    )),
                },
            },
            provenance: Some(OutputProvenance {
                session_id: Some(session_id.to_string()),
                entry_id: None,
                tool_call_id: Some(execution_id),
                source: Some("sandbox_artifact".into()),
            }),
            actions: Vec::new(),
            fallback_text: format!("Generated artifact: {path}"),
        });
    }
}

/// Returns a concise, transport-neutral artifact list for channel delivery.
/// The browser can resolve richer previews, while text surfaces still need an
/// explicit record that the generated files exist and where they live.
pub(crate) fn sandbox_artifact_markdown(
    home: &std::path::Path,
    session_id: &str,
) -> Option<String> {
    let path = home
        .join("sandbox")
        .join("executions")
        .join(format!("{session_id}.jsonl"));
    let text = std::fs::read_to_string(path).ok()?;
    let mut rows = Vec::new();
    for line in text.lines() {
        let Ok(event) = serde_json::from_str::<vak_tools::SandboxEvent>(line) else {
            continue;
        };
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

fn snapshot_inner(
    session_id: &str,
    session: &SessionLog,
    planner: &PresentationPlanner,
    adaptive_library: Option<&vak_presentation::PresentationLibrary>,
) -> OutputTimeline {
    let chain = session.chain_to_root();
    let mut tool_results: HashMap<String, (String, bool)> = HashMap::new();
    let mut tool_inputs: HashMap<String, (String, serde_json::Value)> = HashMap::new();
    let mut turn_outcomes: HashMap<usize, vak_intent::OutcomeSpec> = HashMap::new();
    let mut turn_evaluations: HashMap<usize, String> = HashMap::new();
    let mut turn_evidence_state: HashMap<usize, String> = HashMap::new();
    let mut turn_human_review: HashMap<usize, String> = HashMap::new();
    let mut turn_review_verdict: HashMap<usize, String> = HashMap::new();
    let mut selected_presentation: Option<(String, u64)> = None;
    let mut successful_runs = std::collections::HashSet::new();
    let mut scan_turn = 0usize;
    let mut assistant_tool_context: HashMap<String, TurnTool> = HashMap::new();
    let mut pending_tool_context: Option<TurnTool> = None;
    for entry in &chain {
        match &entry.payload {
            EntryPayload::Message(record) => {
                if record.message.role == Role::User
                    && record.message.content.iter().any(|block| match block {
                        ContentBlock::Text { text } => !clean_scaffolding(text).is_empty(),
                        _ => false,
                    })
                {
                    scan_turn += 1;
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
                    // Core records admission immediately before the user
                    // message that starts the turn. Attach it to that next
                    // turn rather than decorating the previous answer.
                    turn_outcomes.insert(scan_turn + 1, outcome.clone());
                }
            }
            EntryPayload::Activity(activity)
                if activity.kind == ActivityKind::Diagnostic
                    && activity.label == "Outcome evaluation" =>
            {
                if let Some(turn) = activity.turn {
                    if let Some(status) = activity.data.get("status") {
                        turn_evaluations.insert(turn, status.clone());
                    }
                    if let Some(evidence_state) = activity.data.get("evidence_state") {
                        turn_evidence_state.insert(turn, evidence_state.clone());
                    }
                    if let Some(review) = activity.data.get("human_review") {
                        turn_human_review.insert(turn, review.clone());
                    }
                    if let Some(evaluation) = activity.data.get("evaluation") {
                        let evaluation_status = match activity.data.get("status") {
                            Some(value) => value.as_str(),
                            None => "unknown",
                        };
                        turn_evaluations
                            .insert(turn, format!("{}|{}", evaluation_status, evaluation));
                    }
                    if let Some(receipts) = activity.data.get("evidence_receipts") {
                        let evaluation_status = activity
                            .data
                            .get("status")
                            .map_or("unknown", |value| value.as_str());
                        let evaluation = activity
                            .data
                            .get("evaluation")
                            .map_or("[]", |value| value.as_str());
                        let completion = activity
                            .data
                            .get("completion")
                            .map_or("unknown", |value| value.as_str());
                        turn_evaluations.insert(
                            turn,
                            format!(
                                "{}|{}|{}|{}",
                                evaluation_status, evaluation, receipts, completion
                            ),
                        );
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
                if let Some(turn) = activity.turn
                    && let Some(verdict) = activity.data.get("verdict")
                {
                    turn_review_verdict.insert(turn, verdict.clone());
                }
            }
            EntryPayload::Activity(activity)
                if activity.kind == ActivityKind::Run
                    && activity.status == ActivityStatus::Succeeded =>
            {
                if let Some(turn) = activity.turn {
                    successful_runs.insert(turn);
                }
            }
            _ => {}
        }
    }

    let mut timeline = OutputTimeline::empty(session_id);
    timeline.goal = session.goal_state();
    let mut turn = 0usize;
    for entry in chain {
        match &entry.payload {
            EntryPayload::Message(record) => {
                if record.message.role == Role::User
                    && record.message.content.iter().any(|block| match block {
                        ContentBlock::Text { text } => !clean_scaffolding(text).is_empty(),
                        _ => false,
                    })
                {
                    turn += 1;
                }
                let turn_id = format!("turn-{turn}");
                for (index, block) in record.message.content.iter().enumerate() {
                    match block {
                        ContentBlock::Text { text } if !text.trim().is_empty() => {
                            let assistant = record.message.role == Role::Assistant;
                            // A short presentation envelope is ledger metadata, not prose.
                            // Projecting it creates a duplicate, empty-looking Answer card.
                            if assistant && is_presentation_envelope(text) {
                                continue;
                            }
                            let cleaned_text_storage = clean_scaffolding(text);
                            if cleaned_text_storage.is_empty() {
                                continue;
                            }
                            let text = &cleaned_text_storage;
                            let mut candidates = if assistant {
                                structured_outputs_from_text(text)
                            } else {
                                Vec::new()
                            };
                            if assistant {
                                candidates.extend(link_previews_from_text(text));
                            }
                            let assistant_tool_call_id = assistant_tool_context
                                .get(&entry.id)
                                .map(|(id, ..)| id.clone());
                            let (tool_name, tool_input, tool_output, is_error) =
                                assistant_tool_context
                                    .get(&entry.id)
                                    .map(|(_, name, input, output, err)| {
                                        (Some(name.as_str()), Some(input), output.as_deref(), *err)
                                    })
                                    .unwrap_or((None, None, None, false));
                            let ctx = SignalContext {
                                text,
                                tool_name,
                                tool_input,
                                tool_output,
                                is_error,
                            };
                            let signals = signals_from_context(&ctx);
                            let plan = planner.plan(&signals, "desktop", &[], &candidates);
                            let mut projected_outcome = turn_outcomes.get(&turn).cloned();
                            let mut rejected_outcome_requirements = Vec::new();
                            if let Some(outcome) = projected_outcome.as_mut() {
                                rejected_outcome_requirements =
                                    plan.merge_outcome_requirements(outcome);
                            }
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
                                    plan.accepted.iter().find_map(|candidate| {
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
                                    .get(&turn)
                                    .and_then(|value| value.split('|').nth(3)),
                            );
                            timeline.items.push(OutputItem {
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
                                            let available = plan
                                                .accepted
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
                                            if let Some((spec_id, revision)) = selected_for_output
                                                .as_ref()
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
                                        if let Some(decision) = plan.recipe.as_ref() {
                                            document.metadata.insert(
                                                "recipe_id".into(),
                                                decision.recipe_id.clone(),
                                            );
                                            document.metadata.insert(
                                                "recipe_version".into(),
                                                decision.recipe_version.clone(),
                                            );
                                            document.metadata.insert(
                                                "matched_signals".into(),
                                                decision.matched_signals.join(","),
                                            );
                                            document.metadata.insert(
                                                "renderer".into(),
                                                decision.renderer.clone(),
                                            );
                                            document.metadata.insert(
                                                "renderer_blocks".into(),
                                                plan.renderers
                                                    .iter()
                                                    .map(|render| {
                                                        format!(
                                                            "{}={} ({:?})",
                                                            render.semantic_type,
                                                            render.renderer,
                                                            render.disposition
                                                        )
                                                    })
                                                    .collect::<Vec<_>>()
                                                    .join(", "),
                        );
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
                                        if !rejected_outcome_requirements.is_empty() {
                                            document.metadata.insert(
                                                "outcome_requirement_rejections".into(),
                                                rejected_outcome_requirements.join("; "),
                                            );
                                        }
                                        if let Some(status) = turn_evaluations.get(&turn) {
                                            let mut parts = status.splitn(2, '|');
                                            let outcome_status =
                                                parts.next().map_or("unknown", |value| value);
                                            document.metadata.insert(
                                                "outcome_status".into(),
                                                outcome_status.into(),
                                            );
                                            if let Some(evaluation) = parts.next() {
                                                let evaluation_json =
                                                    evaluation.split('|').next().map_or("[]", |value| value);
                                                document.metadata.insert(
                                                    "outcome_evaluation".into(),
                                                    evaluation_json.into(),
                                                );
                                                if let Some(receipts) = evaluation.split('|').nth(2) {
                                                    document.metadata.insert(
                                                        "outcome_evidence_receipts".into(),
                                                        receipts.into(),
                                                    );
                                                }
                                                if let Some(completion) = evaluation.split('|').nth(3)
                                                {
                                                    document.metadata.insert(
                                                        "outcome_completion".into(),
                                                        completion.into(),
                                                    );
                                                }
                                            }
                                        }
                                        if let Some(evidence_state) = turn_evidence_state.get(&turn) {
                                            document.metadata.insert(
                                                "outcome_evidence_state".into(),
                                                evidence_state.clone(),
                                            );
                                        }
                                        if let Some(review) = turn_human_review.get(&turn) {
                                            document.metadata.insert(
                                                "outcome_human_review".into(),
                                                review.clone(),
                                            );
                                        }
                                        if let Some(verdict) = turn_review_verdict.get(&turn) {
                                            document.metadata.insert(
                                                "outcome_review_verdict".into(),
                                                verdict.clone(),
                                            );
                                        }
                                        document.diagnostics.extend(plan.rejected.iter().map(
                                            |item| {
                                                format!("{}: {}", item.semantic_type, item.reason)
                                            },
                                        ));
                                        if assistant
                                            && plan.recipe.as_ref().is_some_and(|decision| {
                                                decision.requires_typed_output
                                            })
                                            && !plan.accepted.iter().any(|candidate| {
                                                plan.recipe.as_ref().is_some_and(|decision| {
                                                    decision.typed_output_types.iter().any(|kind| {
                                                        candidate.semantic_type == *kind
                                                    })
                                                })
                                            })
                                        {
                                            document.diagnostics.push(
                                                "research-shaped prose has no typed evidence contract; rendered as an unverified document".into(),
                                            );
                                        }
                                        document
                                    },
                                },
                                outcome: result_outcome(
                                    format!("{}-text-{index}", entry.id),
                                    output_status,
                                    projected_outcome.as_ref(),
                                    turn_evaluations.get(&turn).map(String::as_str),
                                    turn_evidence_state.get(&turn),
                                    turn_human_review.get(&turn),
                                ),
                                provenance: Some(OutputProvenance {
                                    session_id: Some(session_id.into()),
                                    entry_id: Some(entry.id.clone()),
                                    tool_call_id: assistant_tool_call_id,
                                    source: Some("session_ledger".into()),
                                }),
                                actions: Vec::new(),
                                fallback_text: text.clone(),
                            });
                            if let (Some(library), Some((spec_id, revision))) =
                                (adaptive_library, selected_for_output.as_ref())
                                && let Some(stored) = library.definitions().find(|stored| {
                                    stored.spec.id == *spec_id && stored.spec.revision == *revision
                                })
                                && let Some(candidate) = plan.accepted.iter().find(|candidate| {
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
                                    }),
                                    actions: Vec::new(),
                                    fallback_text: text.clone(),
                                });
                            }
                            for (link_index, preview) in plan
                                .accepted
                                .into_iter()
                                .filter(|candidate| candidate.semantic_type == "link.preview")
                                .enumerate()
                            {
                                timeline.items.push(OutputItem {
                                    id: format!("{}-link-{link_index}", entry.id),
                                    timestamp: entry.ts.to_rfc3339(),
                                    turn_id: turn_id.clone(),
                                    role: OutputRole::Assistant,
                                    kind: OutputKind::Information,
                                    status: OutputStatus::Succeeded,
                                    outcome: None,
                                    content: OutputContent::Structured {
                                        output: preview.clone(),
                                    },
                                    provenance: Some(OutputProvenance {
                                        session_id: Some(session_id.into()),
                                        entry_id: Some(entry.id.clone()),
                                        tool_call_id: None,
                                        source: Some("link_extractor".into()),
                                    }),
                                    actions: Vec::new(),
                                    fallback_text: preview
                                        .payload
                                        .get("url")
                                        .and_then(serde_json::Value::as_str)
                                        .unwrap_or_default()
                                        .into(),
                                });
                            }
                        }
                        ContentBlock::ToolUse { id, name, input } => {
                            let result = tool_results.get(id);
                            let failed = result.is_some_and(|(_, failed)| *failed);
                            let recovered = failed && successful_runs.contains(&turn);
                            let detail = result.map(|(content, _)| content.clone());
                            timeline.items.push(OutputItem {
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
                                for (structured_index, output) in detail
                                    .as_deref()
                                    .map(|text| {
                                        structured_outputs_from_tool_result_with(
                                            text,
                                            "desktop",
                                            &built_in_adapters(),
                                            &planner.skills,
                                        )
                                    })
                                    .unwrap_or_default()
                                    .into_iter()
                                    .enumerate()
                                {
                                    timeline.items.push(OutputItem {
                                        id: format!("{id}-structured-{structured_index}"),
                                        timestamp: entry.ts.to_rfc3339(),
                                        turn_id: turn_id.clone(),
                                        role: OutputRole::Tool,
                                        kind: OutputKind::Information,
                                        status: OutputStatus::Succeeded,
                                        outcome: None,
                                        fallback_text: structured_markdown(&output),
                                        content: OutputContent::Structured { output },
                                        provenance: Some(OutputProvenance {
                                            session_id: Some(session_id.into()),
                                            entry_id: Some(entry.id.clone()),
                                            tool_call_id: Some(id.clone()),
                                            source: Some(name.clone()),
                                        }),
                                        actions: Vec::new(),
                                    });
                                }
                            }
                            if !failed && let Some(artifact) = artifact_from_tool(name, input) {
                                let mut data = BTreeMap::new();
                                if let Some(path) = &artifact.path {
                                    data.insert("path".into(), path.clone());
                                }
                                timeline.items.push(OutputItem {
                                    id: format!("{id}-artifact"),
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
            EntryPayload::Activity(activity) => {
                timeline.items.push(activity_item(
                    session_id,
                    &entry.id,
                    &entry.ts.to_rfc3339(),
                    &format!("turn-{}", activity.turn.unwrap_or(turn)),
                    activity,
                ));
            }
            EntryPayload::GoalUpdate(update) => {
                let label = match update.relation {
                    vak_intent::GoalRelation::New => "Goal started",
                    vak_intent::GoalRelation::AddsTo => "Goal expanded",
                    vak_intent::GoalRelation::Corrects => "Goal corrected",
                    vak_intent::GoalRelation::Replaces => "Goal revised",
                    vak_intent::GoalRelation::Status => "Status requested",
                    vak_intent::GoalRelation::Pauses => "Goal paused",
                    vak_intent::GoalRelation::Resumes => "Goal resumed",
                    vak_intent::GoalRelation::Cancels => "Goal cancelled",
                };
                timeline.items.push(OutputItem {
                    id: format!("goal-update-{}", entry.id),
                    timestamp: entry.ts.to_rfc3339(),
                    turn_id: format!("turn-{turn}"),
                    role: OutputRole::System,
                    kind: OutputKind::Progress,
                    status: match update.relation {
                        vak_intent::GoalRelation::Pauses => OutputStatus::Partial,
                        vak_intent::GoalRelation::Cancels => OutputStatus::Cancelled,
                        _ => OutputStatus::Succeeded,
                    },
                    outcome: None,
                    content: OutputContent::Progress {
                        label: label.into(),
                        detail: Some(update.request.clone()),
                        percent: None,
                    },
                    provenance: Some(OutputProvenance {
                        session_id: Some(session_id.into()),
                        entry_id: Some(entry.id.clone()),
                        tool_call_id: None,
                        source: Some("goal_update".into()),
                    }),
                    actions: Vec::new(),
                    fallback_text: format!("{label}: {}", update.request),
                });
            }
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
    timeline.items = deduplicated;
    timeline.cursor = chain_cursor(session);
    log_turns_with_no_visible_answer(session_id, &timeline);
    timeline
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
fn log_turns_with_no_visible_answer(session_id: &str, timeline: &OutputTimeline) {
    let mut turns: BTreeMap<&str, Vec<&OutputItem>> = BTreeMap::new();
    for item in &timeline.items {
        turns.entry(item.turn_id.as_str()).or_default().push(item);
    }
    for (turn_id, items) in turns {
        let has_real_answer = items.iter().any(|item| match &item.content {
            OutputContent::Document { .. }
            | OutputContent::Structured { .. }
            | OutputContent::Adaptive { .. } => true,
            OutputContent::Outcome { document, .. } => document.is_some(),
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
        let kinds: Vec<String> = non_progress
            .iter()
            .map(|item| format!("{:?}/{:?}", item.kind, item.status))
            .collect();
        eprintln!(
            "[projection] session={session_id} turn={turn_id} produced no visible answer ({} non-progress item(s): {}) — client renders a fallback notice for this turn",
            non_progress.len(),
            kinds.join(", ")
        );
    }
}

fn chain_cursor(session: &SessionLog) -> Option<String> {
    session.chain_to_root().last().map(|entry| entry.id.clone())
}

fn artifact_from_tool(name: &str, input: &serde_json::Value) -> Option<ArtifactRef> {
    if !matches!(name, "write" | "edit" | "apply_patch" | "imagegen") {
        return None;
    }
    let path = ["path", "file_path", "filename", "output_path"]
        .iter()
        .find_map(|key| input.get(*key).and_then(serde_json::Value::as_str))?
        .to_string();
    let filename = std::path::Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&path)
        .to_string();
    Some(ArtifactRef {
        name: filename,
        media_type: media_type_for_path(&path),
        path: Some(path),
        description: Some(format!("Produced by {name}")),
    })
}

fn media_type_for_path(path: &str) -> Option<String> {
    let extension = std::path::Path::new(path).extension()?.to_str()?;
    Some(
        match extension.to_ascii_lowercase().as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            "pdf" => "application/pdf",
            "md" => "text/markdown",
            "json" => "application/json",
            _ => "text/plain",
        }
        .into(),
    )
}

pub(crate) fn is_scaffolding_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("Surface:")
        || trimmed.starts_with("Outcome:")
        || trimmed.starts_with("primary deliverable:")
        || trimmed.eq_ignore_ascii_case("completed")
        || trimmed.starts_with("contract_id:")
        || trimmed.eq_ignore_ascii_case("vak")
        || trimmed.starts_with("[stop-guard]")
        || trimmed.starts_with("[stop-hook]")
        || trimmed.starts_with("[repair directive]")
        || trimmed.starts_with("[recovery]")
        || trimmed.starts_with("[post-tool-use hook]")
        || trimmed.starts_with("I will write and execute this within the sandbox")
}

pub(crate) fn strip_control_blocks(text: &str) -> String {
    let mut out = text.to_string();
    let tags = [
        "conversation_thread",
        "context_summary",
        "intent",
        "work_contract",
        "managed_work",
        "context_packet",
        "system_reminder",
        "runtime_guidance",
        "scratchpad",
    ];
    for tag in tags {
        let open_pattern = format!("<{tag}");
        let close_pattern = format!("</{tag}>");
        while let Some(start) = out.find(&open_pattern) {
            if let Some(end_offset) = out[start..].find(&close_pattern) {
                let end = start + end_offset + close_pattern.len();
                out.replace_range(start..end, "");
            } else {
                out.truncate(start);
                break;
            }
        }
    }

    for prefix in [
        "[stop-guard]:",
        "[stop-hook]:",
        "[repair directive]",
        "[recovery]",
        "[post-tool-use hook]:",
    ] {
        while let Some(start) = out.find(prefix) {
            let remainder = &out[start..];
            if prefix == "[repair directive]" || prefix == "[recovery]" {
                out.truncate(start);
                break;
            }
            if let Some(end_offset) = remainder.find("Please continue.") {
                let end = start + end_offset + "Please continue.".len();
                out.replace_range(start..end, "");
            } else if let Some(end_offset) = remainder.find("Please continue") {
                let end = start + end_offset + "Please continue".len();
                out.replace_range(start..end, "");
            } else if let Some(newline_offset) = remainder.find('\n') {
                let end = start + newline_offset + 1;
                out.replace_range(start..end, "");
            } else {
                out.truncate(start);
                break;
            }
        }
    }

    out
}

pub(crate) fn clean_scaffolding(text: &str) -> String {
    let had_trailing_newline = text.ends_with('\n');
    let stripped = strip_control_blocks(text);
    let mut lines = stripped
        .lines()
        .filter(|line| !is_scaffolding_line(line))
        .collect::<Vec<_>>();
    while let Some(first) = lines.first() {
        if first.trim().is_empty() {
            lines.remove(0);
        } else {
            break;
        }
    }
    while let Some(last) = lines.last() {
        if last.trim().is_empty() {
            lines.pop();
        } else {
            break;
        }
    }
    let mut out = lines.join("\n");
    if had_trailing_newline && !out.is_empty() {
        out.push('\n');
    }
    out
}

fn is_presentation_envelope(text: &str) -> bool {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    !lines.is_empty() && lines.iter().all(|line| is_scaffolding_line(line))
}

fn activity_item(
    session_id: &str,
    entry_id: &str,
    timestamp: &str,
    turn_id: &str,
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
        | ActivityKind::PresentationFeedback => (
            OutputRole::System,
            OutputKind::Information,
            OutputContent::Information {
                label: activity.label.clone(),
                detail: activity.detail.clone(),
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
                OutputContent::Error {
                    message: activity
                        .detail
                        .clone()
                        .unwrap_or_else(|| activity.label.clone()),
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
                if let Some(turn) = activity.turn {
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
        AgentEvent::TurnStart { turn } => Some(OutputStreamEvent::ItemStarted {
            item: OutputItem {
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
                        description: Some(format!(
                            "Generated by sandbox execution ({size_bytes} bytes)"
                        )),
                    },
                },
                provenance: Some(OutputProvenance {
                    session_id: Some(session_id.into()),
                    entry_id: None,
                    tool_call_id: Some(execution_id),
                    source: Some("sandbox_artifact".into()),
                }),
                actions: Vec::new(),
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
        AgentEvent::WorkState { projection } => {
            let status = match projection.status {
                vak_session::types::WorkContractStatus::Completed => OutputStatus::Succeeded,
                vak_session::types::WorkContractStatus::Failed
                | vak_session::types::WorkContractStatus::Cancelled
                | vak_session::types::WorkContractStatus::Unverified => OutputStatus::Failed,
                vak_session::types::WorkContractStatus::Draft
                | vak_session::types::WorkContractStatus::AwaitingInput
                | vak_session::types::WorkContractStatus::Active
                | vak_session::types::WorkContractStatus::Blocked
                | vak_session::types::WorkContractStatus::Verifying => OutputStatus::Running,
            };
            let detail = serde_json::to_string(&projection).unwrap_or_else(|_| "{}".into());
            Some(OutputStreamEvent::ItemReplaced {
                item: live_item(
                    session_id,
                    format!("work-{}", projection.contract.contract_id),
                    now,
                    OutputRole::System,
                    OutputKind::Progress,
                    status,
                    OutputContent::Information {
                        label: "Managed work state".into(),
                        detail: Some(detail.clone()),
                    },
                    detail,
                ),
            })
        }
        AgentEvent::ApprovalRequested {
            id,
            tool,
            args_json,
            reason,
        } => Some(OutputStreamEvent::ItemStarted {
            item: OutputItem {
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
        AgentEvent::RunFinished { is_error, .. } => Some(OutputStreamEvent::ItemCompleted {
            item_id: "live-assistant".into(),
            status: if is_error {
                OutputStatus::Failed
            } else {
                OutputStatus::Succeeded
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
    Some(vak_delivery::OutputStreamFrame {
        sequence: Some(sequence),
        delta: Some(event),
        snapshot: timeline.clone(),
    })
}

pub(crate) fn apply_stream_event(timeline: &mut OutputTimeline, event: OutputStreamEvent) {
    match event {
        OutputStreamEvent::Snapshot { timeline: snapshot } => *timeline = snapshot,
        OutputStreamEvent::ItemStarted { item } | OutputStreamEvent::ItemReplaced { item } => {
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
        OutputStreamEvent::TextDelta { item_id, text, .. } => {
            if let Some(item) = timeline
                .items
                .iter_mut()
                .find(|candidate| candidate.id == item_id)
            {
                item.fallback_text = text.clone();
                item.content = OutputContent::Document {
                    document: compile_markdown(text),
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
        }),
        actions: Vec::new(),
        fallback_text,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::activity_item;
    use super::{artifact_from_tool, snapshot};
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use vak_delivery::{OutputContent, OutputKind, OutputStatus};
    use vak_llm::{ContentBlock, Message, Role};
    use vak_session::{
        ActivityKind, ActivityRecord, ActivityStatus, FrozenContract, MessageRecord, SessionHeader,
        SessionLog,
    };

    #[test]
    fn write_tools_project_artifacts() {
        let artifact = artifact_from_tool(
            "write",
            &serde_json::json!({ "path": "/tmp/report.md", "content": "x" }),
        )
        .expect("write should produce an artifact");
        assert_eq!(artifact.name, "report.md");
        assert_eq!(artifact.media_type.as_deref(), Some("text/markdown"));
    }

    #[test]
    fn outcome_evaluation_exposes_review_actions() {
        let item = activity_item(
            "session-1",
            "activity-1",
            "2026-01-01T00:00:00Z",
            "turn-1",
            &ActivityRecord {
                activity_id: "evaluation-1".into(),
                turn: Some(1),
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
    fn read_tools_do_not_claim_artifacts() {
        assert!(artifact_from_tool("read", &serde_json::json!({ "path": "a" })).is_none());
    }

    #[test]
    fn presentation_envelope_is_not_an_answer() {
        assert!(super::is_presentation_envelope(
            "Outcome: produced\nSurface: desktop app."
        ));
        assert!(!super::is_presentation_envelope(
            "Outcome: produced\n\nHere is the detailed answer."
        ));
    }

    #[test]
    fn snapshot_is_outcome_first_and_deduplicates_activity_transitions() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let mut log = SessionLog::create(
            dir.path().join("presentation.jsonl"),
            SessionHeader {
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
        for status in [ActivityStatus::Pending, ActivityStatus::Succeeded] {
            log.append_activity(ActivityRecord {
                activity_id: "approval-1".into(),
                turn: Some(1),
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
            turn: Some(1),
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
            turn: Some(1),
            kind: ActivityKind::Diagnostic,
            status: ActivityStatus::Succeeded,
            label: "Outcome review".into(),
            detail: None,
            data: BTreeMap::from([(String::from("verdict"), String::from("accepted"))]),
        })
        .expect("append outcome review");
        log.append_activity(ActivityRecord {
            activity_id: "review-2".into(),
            turn: Some(1),
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
        assert!(first.items.iter().any(|item| matches!(
            item.content,
            OutputContent::Structured { ref output } if output.semantic_type == "link.preview"
        )));
    }

    #[test]
    fn recovered_tool_failure_stays_in_activity_without_attention_banner() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let mut log = SessionLog::create(
            dir.path().join("presentation.jsonl"),
            SessionHeader {
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
            turn: Some(1),
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
    fn a_tool_self_declaring_its_own_result_renders_structured_with_no_special_casing() {
        // Any tool — not just ones this file knows by name — gets a rich
        // render for free by tagging its own result with a semantic_type the
        // registry recognizes. This models an arbitrary MCP tool doing that;
        // nothing here mentions "weather" and nothing should have to.
        let dir = tempfile::tempdir().expect("temporary directory");
        let mut log = SessionLog::create(
            dir.path().join("presentation.jsonl"),
            SessionHeader {
                agent: None,
                session_id: "session-3".into(),
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
            message: Message::user_text("What's the weather?"),
            meta: None,
        })
        .expect("append user");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::ToolUse {
                id: "tool-any".into(),
                name: "some_third_party_mcp_tool".into(),
                input: serde_json::json!({}),
            }]),
            meta: None,
        })
        .expect("append call");
        log.append_message(MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "tool-any".into(),
                    content: "```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"Temperature\",\"value\":25,\"unit\":\"C\"}}\n```".into(),
                    is_error: false,
                }],
            },
            meta: None,
        })
        .expect("append result");

        let timeline = snapshot("session-3", &log);
        let structured = timeline
            .items
            .iter()
            .find(|item| matches!(item.content, OutputContent::Structured { .. }))
            .expect("tool result should have produced a structured item");
        let OutputContent::Structured { output } = &structured.content else {
            unreachable!()
        };
        assert_eq!(output.semantic_type, "metric");
        assert_eq!(
            structured
                .provenance
                .as_ref()
                .and_then(|p| p.tool_call_id.as_deref()),
            Some("tool-any")
        );
    }

    /// One end-to-end pass across four unrelated personas' tools, each
    /// returning bare JSON with no Markdown fence — the shape a real tool
    /// implementation naturally produces. No persona, tool name, or domain
    /// is known to `snapshot`; each renders solely because its own result
    /// declared a `semantic_type` the registry recognizes.
    #[test]
    fn different_personas_tools_all_render_through_the_same_unnamed_path() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let mut log = SessionLog::create(
            dir.path().join("presentation.jsonl"),
            SessionHeader {
                agent: None,
                session_id: "session-personas".into(),
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
            message: Message::user_text("Kick off a bunch of unrelated tools"),
            meta: None,
        })
        .expect("append user");

        // (tool name, persona it stands in for, bare-JSON result, expected semantic_type)
        let calls: [(&str, &str, &str, &str); 4] = [
            (
                "get_local_weather",
                "general user",
                r#"{"semantic_type":"metric","payload":{"label":"Temperature","value":25,"unit":"C"}}"#,
                "metric",
            ),
            (
                "run_test_suite",
                "developer",
                r#"{"semantic_type":"test.report","payload":{"tests":[{"name":"it_compiles","status":"passed"}],"total":1,"passed":1,"failed":0,"skipped":0}}"#,
                "test.report",
            ),
            (
                "aggregate_research",
                "knowledge worker",
                r#"{"semantic_type":"research.synthesis","payload":{"sources":[{"title":"Report","url":"https://example.com"}],"takeaways":[{"text":"Adoption is rising","citation_indices":[1]}]}}"#,
                "research.synthesis",
            ),
            (
                "query_warehouse",
                "data analyst",
                r#"{"semantic_type":"data.grid","payload":{"columns":[{"key":"region","label":"Region"}],"rows":[{"region":"APAC"}]}}"#,
                "data.grid",
            ),
        ];

        for (index, (tool_name, _persona, result_json, _expected_type)) in calls.iter().enumerate()
        {
            let call_id = format!("call-{index}");
            log.append_message(MessageRecord {
                message: Message::assistant(vec![ContentBlock::ToolUse {
                    id: call_id.clone(),
                    name: (*tool_name).into(),
                    input: serde_json::json!({}),
                }]),
                meta: None,
            })
            .expect("append call");
            log.append_message(MessageRecord {
                message: Message {
                    role: Role::User,
                    content: vec![ContentBlock::ToolResult {
                        tool_use_id: call_id,
                        content: (*result_json).into(),
                        is_error: false,
                    }],
                },
                meta: None,
            })
            .expect("append result");
        }

        let timeline = snapshot("session-personas", &log);
        for (index, (_tool_name, persona, _result_json, expected_type)) in calls.iter().enumerate()
        {
            let call_id = format!("call-{index}");
            let found = timeline.items.iter().any(|item| {
                matches!(&item.content, OutputContent::Structured { output }
                    if output.semantic_type == *expected_type)
                    && item
                        .provenance
                        .as_ref()
                        .and_then(|p| p.tool_call_id.as_deref())
                        == Some(call_id.as_str())
            });
            assert!(
                found,
                "expected a rendered {expected_type} item for the {persona} scenario"
            );
        }
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
    fn clean_scaffolding_strips_stop_hooks_and_scaffolding() {
        let text =
            "[stop-hook]: continue required by hook\nPlease continue.\nHere is the real answer.";
        let cleaned = super::clean_scaffolding(text);
        assert_eq!(cleaned, "Here is the real answer.");

        let text2 = "[stop-guard]: goal not met\nPlease continue.\nSurface: desktop app\nDone.";
        let cleaned2 = super::clean_scaffolding(text2);
        assert_eq!(cleaned2, "Done.");

        let text3 = "<intent>select</intent><context_packet>data</context_packet>Final result.";
        let cleaned3 = super::clean_scaffolding(text3);
        assert_eq!(cleaned3, "Final result.");
    }

    #[test]
    fn synthetic_stop_messages_do_not_increment_turn_or_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = SessionLog::create(
            dir.path().join("synthetic-turns.jsonl"),
            SessionHeader {
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
            message: Message::user_text("[stop-hook]: hook said continue\nPlease continue."),
            meta: None,
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
            message: Message::user_text("[repair directive] The run is stuck on correctable tool failures...\nAdmitted tools: read"),
            meta: None,
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
}
