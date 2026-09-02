use std::collections::{BTreeMap, HashMap};

use vak_agent::AgentEvent;
use vak_delivery::{
    ArtifactRef, DeliveryAction, OutputContent, OutputItem, OutputKind, OutputProvenance,
    OutputRole, OutputStatus, OutputStreamEvent, OutputTimeline, built_in_recipes,
    built_in_skill_registry, compile_markdown, link_previews_from_text, signals_from_text,
};
use vak_llm::{ContentBlock, Role};
use vak_session::{ActivityKind, ActivityStatus, EntryPayload, SessionLog};

pub(crate) fn snapshot(session_id: &str, session: &SessionLog) -> OutputTimeline {
    let chain = session.chain_to_root();
    let mut tool_results: HashMap<String, (String, bool)> = HashMap::new();
    let mut successful_runs = std::collections::HashSet::new();
    for entry in &chain {
        match &entry.payload {
            EntryPayload::Message(record) => {
                for block in &record.message.content {
                    if let ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } = block
                    {
                        tool_results.insert(tool_use_id.clone(), (content.clone(), *is_error));
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
                            let link_previews = if assistant {
                                link_previews_from_text(text)
                            } else {
                                Vec::new()
                            };
                            let planner = vak_delivery::PresentationPlanner {
                                skills: built_in_skill_registry(),
                                recipes: built_in_recipes(),
                            };
                            let signals = signals_from_text(text);
                            let plan = planner.plan(&signals, "desktop", &[], &link_previews);
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
                            for (link_index, preview) in plan.accepted.into_iter().enumerate() {
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
                                    .unwrap_or_else(|| format!("{name} completed")),
                            });
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
        AgentEvent::Stream(vak_llm::StreamEvent::TextDelta { delta, .. }) => {
            Some(OutputStreamEvent::TextDelta {
                item_id: "live-assistant".into(),
                delta,
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
        OutputStreamEvent::TextDelta { item_id, delta } => {
            if let Some(item) = timeline
                .items
                .iter_mut()
                .find(|candidate| candidate.id == item_id)
            {
                item.fallback_text.push_str(&delta);
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
}
