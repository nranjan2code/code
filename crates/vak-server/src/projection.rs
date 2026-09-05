use std::collections::{BTreeMap, HashMap};

use vak_agent::AgentEvent;
use vak_delivery::{
    ArtifactRef, DeliveryAction, OutputContent, OutputItem, OutputKind, OutputProvenance,
    OutputRole, OutputStatus, OutputStreamEvent, OutputTimeline, PresentationPlanner,
    SignalContext, built_in_adapters, compile_markdown, link_previews_from_text,
    signals_from_context, structured_markdown, structured_outputs_from_text,
    structured_outputs_from_tool_result,
};
use vak_llm::{ContentBlock, Role};
use vak_session::{ActivityKind, ActivityStatus, EntryPayload, SessionLog};

/// One tool call as the timeline needs it: name, input, result text, and
/// whether the result was an error. Named because the inline tuple was wide
/// enough that a reader had to count commas to find the error flag.
type TurnTool = (String, serde_json::Value, Option<String>, bool);

pub(crate) fn snapshot(session_id: &str, session: &SessionLog) -> OutputTimeline {
    let builtin = PresentationPlanner {
        skills: vak_delivery::built_in_skill_registry(),
        recipes: vak_delivery::built_in_recipes(),
    };
    snapshot_inner(session_id, session, &builtin)
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
    snapshot_inner(session_id, session, planner)
}

fn snapshot_inner(
    session_id: &str,
    session: &SessionLog,
    planner: &PresentationPlanner,
) -> OutputTimeline {
    let chain = session.chain_to_root();
    let mut tool_results: HashMap<String, (String, bool)> = HashMap::new();
    let mut tool_inputs: HashMap<String, (String, serde_json::Value)> = HashMap::new();
    let mut successful_runs = std::collections::HashSet::new();
    let mut scan_turn = 0usize;
    let mut turn_tools: HashMap<usize, Vec<TurnTool>> = HashMap::new();
    for entry in &chain {
        match &entry.payload {
            EntryPayload::Message(record) => {
                if record.message.role == Role::User
                    && record
                        .message
                        .content
                        .iter()
                        .any(|block| matches!(block, ContentBlock::Text { .. }))
                {
                    scan_turn += 1;
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
                                turn_tools.entry(scan_turn).or_default().push((
                                    name.clone(),
                                    input.clone(),
                                    Some(content.clone()),
                                    *is_error,
                                ));
                            }
                        }
                        _ => {}
                    }
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
    let mut turn = 0usize;
    for entry in chain {
        match &entry.payload {
            EntryPayload::Message(record) => {
                if record.message.role == Role::User
                    && record
                        .message
                        .content
                        .iter()
                        .any(|block| matches!(block, ContentBlock::Text { .. }))
                {
                    turn += 1;
                }
                let turn_id = format!("turn-{turn}");
                for (index, block) in record.message.content.iter().enumerate() {
                    match block {
                        ContentBlock::Text { text } if !text.trim().is_empty() => {
                            let assistant = record.message.role == Role::Assistant;
                            let mut candidates = if assistant {
                                structured_outputs_from_text(text)
                            } else {
                                Vec::new()
                            };
                            if assistant {
                                candidates.extend(link_previews_from_text(text));
                            }
                            let (tool_name, tool_input, tool_output, is_error) = turn_tools
                                .get(&turn)
                                .and_then(|tools| tools.last())
                                .map(|(name, input, output, err)| {
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
                                status: OutputStatus::Succeeded,
                                content: OutputContent::Document {
                                    document: {
                                        let mut document = compile_markdown(text.clone());
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
                                        }
                                        document.diagnostics.extend(plan.rejected.iter().map(
                                            |item| {
                                                format!("{}: {}", item.semantic_type, item.reason)
                                            },
                                        ));
                                        document
                                    },
                                },
                                provenance: Some(OutputProvenance {
                                    session_id: Some(session_id.into()),
                                    entry_id: Some(entry.id.clone()),
                                    tool_call_id: None,
                                    source: Some("session_ledger".into()),
                                }),
                                actions: Vec::new(),
                                fallback_text: text.clone(),
                            });
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
                                        structured_outputs_from_tool_result(
                                            text,
                                            "desktop",
                                            &built_in_adapters(),
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
    timeline
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
        ActivityKind::RouteFallback | ActivityKind::Diagnostic => (
            OutputRole::System,
            OutputKind::Information,
            OutputContent::Information {
                label: activity.label.clone(),
                detail: activity.detail.clone(),
            },
        ),
        ActivityKind::Subagent => (
            OutputRole::Subagent,
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
    let actions = if kind == OutputKind::Approval && status == OutputStatus::Pending {
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
        AgentEvent::SubagentStarted { label } => Some(OutputStreamEvent::ItemStarted {
            item: live_item(
                session_id,
                format!("subagent-{label}"),
                now,
                OutputRole::Subagent,
                OutputKind::Progress,
                OutputStatus::Running,
                OutputContent::Progress {
                    label: label.clone(),
                    detail: Some("Subagent started".into()),
                    percent: None,
                },
                format!("{label} started"),
            ),
        }),
        AgentEvent::SubagentFinished {
            label,
            is_error,
            elapsed_ms,
        } => Some(OutputStreamEvent::ItemReplaced {
            item: live_item(
                session_id,
                format!("subagent-{label}"),
                now,
                OutputRole::Subagent,
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
                        source: Some("subagent".into()),
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
    fn read_tools_do_not_claim_artifacts() {
        assert!(artifact_from_tool("read", &serde_json::json!({ "path": "a" })).is_none());
    }

    #[test]
    fn snapshot_is_outcome_first_and_deduplicates_activity_transitions() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let mut log = SessionLog::create(
            dir.path().join("presentation.jsonl"),
            SessionHeader {
                session_id: "session-1".into(),
                created_at: chrono::Utc::now(),
                cwd: PathBuf::from("/tmp/project"),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
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
        assert_eq!(
            document.metadata.get("recipe_id").map(String::as_str),
            Some("answer.research")
        );
        assert_eq!(
            document.metadata.get("renderer").map(String::as_str),
            Some("builtin:generic")
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
                session_id: "session-2".into(),
                created_at: chrono::Utc::now(),
                cwd: PathBuf::from("/tmp/project"),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
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
                session_id: "session-3".into(),
                created_at: chrono::Utc::now(),
                cwd: PathBuf::from("/tmp/project"),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
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
                session_id: "session-personas".into(),
                created_at: chrono::Utc::now(),
                cwd: PathBuf::from("/tmp/project"),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
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
                r#"{"semantic_type":"research.synthesis","payload":{"sources":[{"title":"Report","url":"https://example.com"}],"takeaways":["Adoption is rising"]}}"#,
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
}
