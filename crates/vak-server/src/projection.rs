use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

pub(crate) use vak_intent::control::{clean_scaffolding, is_scaffolding_line};

use vak_agent::AgentEvent;
use vak_delivery::{
    ArtifactRef, ArtifactStatus, DeliveryAction, OutputContent, OutputItem, OutputKind,
    OutputProvenance, OutputRole, OutputStatus, OutputStreamEvent, OutputTimeline,
    PresentationDocument, PresentationPlanner, ResultOutcome, SignalContext, built_in_adapters,
    compile_markdown, link_previews_from_text, signals_from_context, structured_markdown,
    structured_outputs_from_text, structured_outputs_from_tool_result_with,
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
) -> ResultOutcome {
    let completion = evaluation
        .and_then(|value| value.split('|').nth(3))
        .map(str::to_owned);
    ResultOutcome {
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
    match vak_delivery::supplemental_card_note(&narration) {
        Some(note) => format!("{cards}\n\n{note}"),
        None => cards,
    }
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
        .filter(|item| item.role == OutputRole::Assistant)
        .filter_map(|item| {
            item.outcome
                .clone()
                .map(|outcome| (item.turn_id.clone(), outcome))
        })
        .collect();
    let path = home
        .join("sandbox")
        .join("executions")
        .join(format!("{session_id}.jsonl"));
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let events = text
        .lines()
        .filter_map(|line| serde_json::from_str::<vak_tools::SandboxEvent>(line).ok())
        .collect::<Vec<_>>();
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
    let drafts = vak_sandbox::load_records(&home.join("sandbox").join("records.jsonl"))
        .ok()
        .map(|records| DraftVersions::new(records, session_id));
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
        // Only an execution that ran inside `.vak/scratch/` holds a draft to
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
        // result presents one file and one Canvas entry point.
        if let Some(existing) = timeline.items.iter_mut().find(|item| {
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
        }) {
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

/// `path` (absolute, or relative to the workspace) relative to the scratch
/// directory it was written in, which is how a saved version names it.
fn draft_relative_path<'a>(path: &'a str, scratch: &str) -> Option<&'a std::path::Path> {
    let artifact = std::path::Path::new(path);
    let scratch = std::path::Path::new(scratch);
    if artifact.is_absolute() {
        return artifact.strip_prefix(scratch).ok();
    }
    let workspace = scratch
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == ".vak"))
        .and_then(std::path::Path::parent)?;
    artifact
        .strip_prefix(scratch.strip_prefix(workspace).ok()?)
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

/// Declared domains for `tool_name`, accumulated from the chain's
/// `TurnCapabilitiesBound` entries — never guessed from the tool's name
/// (docs/design/68-context-engine.md §9's `SignalContext.domains` note).
fn tool_domain_refs<'a>(
    tool_domains: &'a HashMap<String, Vec<String>>,
    tool_name: Option<&str>,
) -> Vec<&'a str> {
    tool_name
        .and_then(|name| tool_domains.get(name))
        .map(|domains| domains.iter().map(String::as_str).collect())
        .unwrap_or_default()
}

fn snapshot_inner(
    session_id: &str,
    session: &SessionLog,
    planner: &PresentationPlanner,
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
            | vak_session::types::PresentationSource::Delegated { .. } => None,
        })
        .collect();
    // A delegated call (`task`) may carry several cards: the ones its worker
    // showed, recorded in this ledger when the worker ended.
    let mut delegated_by_tool_use_id: HashMap<
        String,
        Vec<(String, &vak_session::types::PresentationRecord)>,
    > = HashMap::new();
    for (entry_id, record) in session.presentations() {
        if let vak_session::types::PresentationSource::Delegated { tool_use_id, .. } =
            &record.source
        {
            delegated_by_tool_use_id
                .entry(tool_use_id.clone())
                .or_default()
                .push((entry_id, record));
        }
    }
    let mut tool_results: HashMap<String, (String, bool)> = HashMap::new();
    let mut tool_inputs: HashMap<String, (String, serde_json::Value)> = HashMap::new();
    let mut turn_outcomes: HashMap<usize, vak_intent::OutcomeSpec> = HashMap::new();
    let mut turn_evaluations: HashMap<usize, String> = HashMap::new();
    let mut turn_evidence_state: HashMap<usize, String> = HashMap::new();
    let mut turn_human_review: HashMap<usize, String> = HashMap::new();
    let mut turn_review_verdict: HashMap<usize, String> = HashMap::new();
    // Declared domains per tool name, accumulated from every
    // `TurnCapabilitiesBound` entry in the chain (docs/design/68-context-
    // engine.md §9's `SignalContext.domains` note): delivery signals derive
    // from what a capability declared it serves, never from its name.
    let mut tool_domains: HashMap<String, Vec<String>> = HashMap::new();
    let mut selected_presentation: Option<(String, u64)> = None;
    let mut successful_runs = std::collections::HashSet::new();
    let mut scan_turn = 0usize;
    let mut assistant_tool_context: HashMap<String, TurnTool> = HashMap::new();
    let mut pending_tool_context: Option<TurnTool> = None;
    // Tracks, per semantic_type, the id of the most recently pushed
    // tool-emitted `Structured` card within the CURRENT logical answer —
    // reset on a genuine new user request. Paired with `repair_armed`
    // below: a same-type card is only ever superseded (not just recorded)
    // while armed, i.e. strictly after a `[fence-check]`/
    // `[duplicate-card-check]` repair-nudge fired for this answer. Without
    // that guard, two intentionally distinct same-type cards the model
    // emits back-to-back in one turn (e.g. "here's revenue, and here's
    // cost", both `chart`) would be wrongly collapsed to one — nothing
    // else in this projection distinguishes "two calls in one batch" from
    // "a retry of the same call".
    //
    // Why this exists: a weak/small local model sometimes "retries" a
    // repair nudge by calling the same `emit_*_card` tool again rather
    // than only fixing its prose (observed live against gemma4:e2b-mlx),
    // which otherwise leaves two separate `Structured` items for what the
    // user experiences as one card. See `ids_to_remove` below: the earlier
    // attempt is dropped in favor of the retry's result, mirroring
    // `vak-agent`'s own bounded repair-turn semantics (the model's LATEST
    // attempt is authoritative).
    let mut card_group_by_type: HashMap<String, String> = HashMap::new();
    let mut seen_cards: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut repair_armed = false;
    let mut ids_to_remove: std::collections::HashSet<String> = std::collections::HashSet::new();
    for entry in &chain {
        match &entry.payload {
            EntryPayload::Message(record) => {
                // What the user wrote versus what the runtime authored is a
                // typed fact on the record (`vak_intent::control`), not
                // something to re-derive from the text.
                let control = record.control_kind();
                if record.message.role == Role::User
                    && control.is_none()
                    && record.message.content.iter().any(|block| match block {
                        ContentBlock::Text { text } => !clean_scaffolding(text).is_empty(),
                        _ => false,
                    })
                {
                    scan_turn += 1;
                    pending_tool_context = None;
                    card_group_by_type.clear();
                    seen_cards.clear();
                    repair_armed = false;
                }
                if control.is_some_and(|kind| kind.retries_answer()) {
                    repair_armed = true;
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
            EntryPayload::TurnCapabilitiesBound(bound) => {
                for (name, domains) in &bound.tool_domains {
                    tool_domains
                        .entry(name.clone())
                        .or_insert_with(|| domains.clone());
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
    // An answer the runtime sent back for a redo (a `retries_answer` nudge
    // followed it) is an internal draft, not something to show: the user sees
    // the redone answer, never both. `last_assistant_entry` is the answer the
    // next such nudge would be rejecting.
    let mut last_assistant_entry: Option<String> = None;
    let mut rejected_drafts: std::collections::HashSet<String> = std::collections::HashSet::new();
    for entry in chain {
        match &entry.payload {
            EntryPayload::Message(record) => {
                let control = record.control_kind();
                if control.is_some_and(|kind| kind.retries_answer())
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
                }
                if record.message.role == Role::User
                    && control.is_none()
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
                            if control.is_some() {
                                continue;
                            }
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
                            let tool_domain_refs = tool_domain_refs(&tool_domains, tool_name);
                            let ctx = SignalContext {
                                text,
                                tool_name,
                                tool_input,
                                tool_output,
                                is_error,
                                domains: &tool_domain_refs,
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
                                            .filter(|stored| {
                                                stored.spec.metadata.get("seed").map(String::as_str)
                                                    != Some("true")
                                                    || stored
                                                        .spec
                                                        .metadata
                                                        .get("certified")
                                                        .map(String::as_str)
                                                        == Some("true")
                                            })
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
                                outcome: Some(result_outcome(
                                    format!("{}-text-{index}", entry.id),
                                    output_status,
                                    projected_outcome.as_ref(),
                                    turn_evaluations.get(&turn).map(String::as_str),
                                    turn_evidence_state.get(&turn),
                                    turn_human_review.get(&turn),
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
                                        presentation_id: None,
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
                                    kind: OutputKind::Card,
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
                                        presentation_id: None,
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
                                // An `emit_*_card` call's card is read from
                                // its own `Presentation` ledger entry
                                // (docs/design/68-context-engine.md §10),
                                // keyed by this call's `tool_use_id` —
                                // written once, at validation, and never
                                // rebuilt from the call's arguments here.
                                let name_is_card = vak_core::presentation_tools::is_card_tool(name);
                                let recorded = |(entry_id, record): &(
                                    String,
                                    &vak_session::types::PresentationRecord,
                                )| {
                                    (
                                        vak_delivery::StructuredOutput {
                                            semantic_type: record.semantic_type.clone(),
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
                                    if name_is_card {
                                        presentation_by_tool_use_id
                                            .get(id)
                                            .map(|(entry_id, record)| {
                                                recorded(&(entry_id.clone(), *record))
                                            })
                                            .into_iter()
                                            .collect()
                                    } else {
                                        let mut outputs: Vec<_> = delegated_by_tool_use_id
                                            .get(id)
                                            .map(|cards| cards.iter().map(recorded).collect())
                                            .unwrap_or_default();
                                        outputs.extend(
                                            detail
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
                                                .map(|output| (output, None)),
                                        );
                                        outputs
                                    };
                                for (structured_index, (output, presentation_entry_id)) in
                                    outputs.into_iter().enumerate()
                                {
                                    // The same card emitted twice in one answer is one
                                    // card, armed retry or not: identical content has
                                    // nothing to supersede and nothing to add.
                                    if name_is_card
                                        && !seen_cards.insert(
                                            serde_json::to_string(&output).unwrap_or_default(),
                                        )
                                    {
                                        continue;
                                    }
                                    let item_id = format!("{id}-structured-{structured_index}");
                                    let previous = card_group_by_type
                                        .insert(output.semantic_type.clone(), item_id.clone());
                                    if repair_armed && let Some(superseded) = previous {
                                        ids_to_remove.insert(superseded);
                                    }
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
                                    let adaptive = selected
                                        .filter(|stored| {
                                            stored.spec.metadata.get("seed").map(String::as_str)
                                                != Some("true")
                                                || stored
                                                    .spec
                                                    .metadata
                                                    .get("certified")
                                                    .map(String::as_str)
                                                    == Some("true")
                                        })
                                        .and_then(|stored| {
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
                timeline.items.push(activity_item(
                    session_id,
                    &entry.id,
                    &entry.ts.to_rfc3339(),
                    &format!("turn-{}", activity.turn.unwrap_or(turn)),
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
    if !ids_to_remove.is_empty() {
        deduplicated.retain(|item| !ids_to_remove.contains(&item.id));
    }
    deduplicate_file_artifacts(&mut deduplicated);
    let card_turns: std::collections::HashSet<String> = deduplicated
        .iter()
        .filter(|item| {
            item.kind == OutputKind::Card
                && item
                    .provenance
                    .as_ref()
                    .and_then(|p| p.source.as_deref())
                    .is_some_and(|source| source.starts_with("emit_") && source.ends_with("_card"))
        })
        .map(|item| item.turn_id.clone())
        .collect();
    for item in &mut deduplicated {
        if !card_turns.contains(&item.turn_id) || item.role != OutputRole::Assistant {
            continue;
        }
        let document = match &mut item.content {
            OutputContent::Document { document } => Some(document),
            OutputContent::Outcome { document, .. } => document.as_mut(),
            _ => None,
        };
        if let Some(document) = document
            && let Some(note) = vak_delivery::supplemental_card_note(&document.source_markdown)
        {
            document.metadata.insert("card_note".into(), note.into());
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
    let name = vak_tools::canonical_tool_name(name);
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
        description: None,
        size_bytes: None,
        status: Some(ArtifactStatus::InFolder),
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

fn is_presentation_envelope(text: &str) -> bool {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    !lines.is_empty() && lines.iter().all(|line| is_scaffolding_line(line))
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
        | ActivityKind::CandidateRevision => true,
        ActivityKind::Run => activity.label != "Request accepted",
        _ => false,
    }
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
    use super::{artifact_from_tool, snapshot};
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
        let skills = vak_delivery::built_in_skill_registry();
        let info = vak_core::presentation_tools::presentation_info(tool, args, &skills)
            .expect("fixture call must validate");
        let turn_id = log.latest_directive_entry_id().unwrap_or_default();
        let payload_digest = vak_session::types::payload_digest(&info.payload);
        log.append_presentation(vak_session::types::PresentationRecord {
            turn_id,
            source: vak_session::types::PresentationSource::ToolCall {
                tool_use_id: tool_use_id.into(),
            },
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
            turn: None,
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
                &serde_json::json!({"semantic_type":"chart","payload":{"chart_type":"line","series":[],"accessible_summary":summary}}),
                &skills,
            )
            .expect("fixture card validates");
            let entry = log
                .append_presentation(vak_session::types::PresentationRecord {
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

    #[test]
    fn write_tools_project_artifacts() {
        let artifact = artifact_from_tool(
            "write",
            &serde_json::json!({ "path": "/tmp/report.md", "content": "x" }),
        )
        .expect("write should produce an artifact");
        assert_eq!(artifact.name, "report.md");
        assert_eq!(artifact.media_type.as_deref(), Some("text/markdown"));

        let html = artifact_from_tool(
            "write_file",
            &serde_json::json!({ "path": "welcome.html", "content": "<h1>Hello</h1>" }),
        )
        .expect("write alias should produce an artifact");
        assert_eq!(html.media_type.as_deref(), Some("text/html"));
    }

    #[test]
    fn sandbox_artifacts_rejoin_their_durable_result_turn() {
        let home = tempfile::tempdir().expect("temporary home");
        let events = home.path().join("sandbox/executions");
        std::fs::create_dir_all(&events).expect("execution directory");
        let scratch = home.path().join(".vak/scratch/call-1");
        std::fs::create_dir_all(&scratch).expect("scratch directory");
        let event = vak_tools::SandboxEvent::ArtifactGenerated {
            execution_id: "call-1".into(),
            path: ".vak/scratch/call-1/report.html".into(),
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
        std::fs::write(
            events.join("session-1.jsonl"),
            format!(
                "{}\n{}\n",
                serde_json::to_string(&started).expect("started json"),
                serde_json::to_string(&event).expect("event json")
            ),
        )
        .expect("event sidecar");

        let mut timeline = OutputTimeline::empty("session-1");
        timeline.items.push(OutputItem {
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
        std::fs::write(
            events.join("session-1.jsonl"),
            format!(
                "{}\n{}\n",
                serde_json::to_string(&started).expect("started json"),
                serde_json::to_string(&generated).expect("event json")
            ),
        )
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
            let text: String = records.iter().map(|record| format!("{record}\n")).collect();
            std::fs::write(home.path().join("sandbox/records.jsonl"), text).expect("records");
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

        std::fs::write(home.path().join("sandbox/records.jsonl"), "not json\n").expect("records");
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
        std::fs::write(
            events.join("session-1.jsonl"),
            format!(
                "{}\n",
                serde_json::to_string(&observed).expect("event json")
            ),
        )
        .expect("event sidecar");
        let mut timeline = OutputTimeline::empty("session-1");
        timeline.items.push(OutputItem {
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
    fn repeated_writes_keep_one_latest_artifact_per_turn_and_path() {
        let artifact = |id: &str, turn: &str, path: &str| OutputItem {
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
    fn tool_domain_refs_reads_declared_domains_by_tool_name() {
        let mut domains = HashMap::new();
        domains.insert(
            "tavily_search".to_string(),
            vec!["web".to_string(), "live-data".to_string()],
        );
        assert_eq!(
            super::tool_domain_refs(&domains, Some("tavily_search")),
            vec!["web", "live-data"]
        );
        assert!(super::tool_domain_refs(&domains, Some("bash")).is_empty());
        assert!(super::tool_domain_refs(&domains, None).is_empty());
    }

    /// `snapshot` must read back a `TurnCapabilitiesBound` entry's declared
    /// `tool_domains` without disturbing the rest of the projection — the
    /// ledger-round-trip half of docs/design/68-context-engine.md §9's
    /// `SignalContext.domains` note. The signal/recipe consequence of a
    /// non-empty `domains` slice is covered directly against
    /// `signals_from_context` in `vak-delivery`.
    #[test]
    fn snapshot_reads_declared_tool_domains_from_the_ledger_without_disrupting_projection() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let mut log = SessionLog::create(
            dir.path().join("presentation.jsonl"),
            SessionHeader {
                space: None,
                run: None,
                cause: None,
                agent: None,
                session_id: "session-domains".into(),
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
        log.append_turn_capabilities(vak_session::types::TurnCapabilitiesBound {
            epoch: 1,
            capability_ids: Vec::new(),
            excluded_ids: Vec::new(),
            system_prompt: String::new(),
            tool_schemas: Vec::new(),
            core_tool_names: Vec::new(),
            deferred_tool_names: Vec::new(),
            tool_index: String::new(),
            tool_domains: BTreeMap::from([(
                "some_search_tool".to_string(),
                vec!["web".to_string(), "live-data".to_string()],
            )]),
        })
        .expect("append turn capabilities");
        log.append_message(MessageRecord {
            message: Message::user_text("What is happening in the market today?"),
            meta: None,
        })
        .expect("append user");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::ToolUse {
                id: "call-1".into(),
                name: "some_search_tool".into(),
                input: serde_json::json!({"query": "market news"}),
            }]),
            meta: None,
        })
        .expect("append call");
        log.append_message(MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![ContentBlock::tool_result("call-1", "found three articles")],
            },
            meta: None,
        })
        .expect("append result");
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::text("Markets are up today.")]),
            meta: None,
        })
        .expect("append narration");

        // Must not panic, and the ordinary answer must still project.
        let timeline = snapshot("session-domains", &log);
        assert!(
            timeline
                .items
                .iter()
                .any(|item| item.kind == OutputKind::Outcome),
            "the turn's answer must still project with a declared-domains entry in the chain"
        );
        assert!(
            timeline.items.iter().any(|item| {
                item.role == OutputRole::Assistant
                    && item.kind == OutputKind::Outcome
                    && item.outcome.as_ref().is_some_and(|outcome| {
                        !outcome.result_id.is_empty() && outcome.status == OutputStatus::Succeeded
                    })
            }),
            "every ordinary assistant result needs a stable address in an unbounded timeline"
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
                space: None,
                run: None,
                cause: None,
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
                space: None,
                run: None,
                cause: None,
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
    fn clean_scaffolding_strips_inline_hints_and_scaffolding() {
        let text = "Answer body.\n[recovery] retry the failing call";
        assert_eq!(super::clean_scaffolding(text), "Answer body.");

        let text2 = "Surface: desktop app\nDone.";
        assert_eq!(super::clean_scaffolding(text2), "Done.");

        let text3 = "<intent>select</intent><context_packet>data</context_packet>Final result.";
        assert_eq!(super::clean_scaffolding(text3), "Final result.");
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
            message: Message::user_text("[stop-hook]: hook said continue\nPlease continue."),
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

    /// Regression coverage for a verified real bug: `gemma4:e2b-mlx`
    /// (via Ollama) called `emit_chart_card` successfully, then vak-agent's
    /// fence-check repair asked it to fix a malformed fence in its
    /// following text, and instead of just editing the text it called
    /// `emit_chart_card` AGAIN — leaving two separate `Structured` tool
    /// results for what the user experiences as one card (confirmed via
    /// `vak export`/direct `snapshot()` against the real session). A
    /// `[fence-check]` (or `[duplicate-card-check]`) nudge marks a retry of
    /// the SAME answer, not a new user request, so a second same-type card
    /// after one of these markers must supersede the earlier one rather
    /// than both surviving into the timeline.
    #[test]
    fn a_retried_tool_card_after_a_repair_nudge_supersedes_the_earlier_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = SessionLog::create(
            dir.path().join("duplicate-card.jsonl"),
            SessionHeader {
                space: None,
                run: None,
                cause: None,
                agent: None,
                session_id: "duplicate-card".into(),
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
            message: Message::user_text("show me a chart"),
            meta: None,
        })
        .expect("append user message");

        let chart_input = |summary: &str| serde_json::json!({"semantic_type":"chart","payload":{"chart_type":"line","series":[],"accessible_summary":summary}});
        let ack = || "Card displayed to the user (chart).".to_string();

        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::ToolUse {
                id: "call-1".into(),
                name: "emit_chart_card".into(),
                input: chart_input("first attempt"),
            }]),
            meta: None,
        })
        .expect("append call 1");
        log.append_message(MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "call-1".into(),
                    content: ack(),
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
            &chart_input("first attempt"),
        );
        log.append_message(MessageRecord {
            message: Message {
                role: Role::Assistant,
                content: vec![ContentBlock::Text {
                    text: "```vak\n{broken json\n```".into(),
                }],
            },
            meta: None,
        })
        .expect("append malformed fence");

        // The repair nudge: same logical answer, not a new user request.
        log.append_message(MessageRecord {
            message: Message::user_text(
                "[fence-check]: The ```vak card block in your last answer has invalid JSON and failed to parse. Resend it.",
            ),
            meta: Some(vak_session::MessageMeta {
                control: Some(vak_intent::control::ControlKind::FenceCheck),
                ..Default::default()
            }),
        })
        .expect("append fence-check nudge");

        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::ToolUse {
                id: "call-2".into(),
                name: "emit_chart_card".into(),
                input: chart_input("retried attempt"),
            }]),
            meta: None,
        })
        .expect("append call 2");
        log.append_message(MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "call-2".into(),
                    content: ack(),
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
            &chart_input("retried attempt"),
        );
        log.append_message(MessageRecord {
            message: Message {
                role: Role::Assistant,
                content: vec![ContentBlock::Text {
                    text: "Chart is displayed above.".into(),
                }],
            },
            meta: None,
        })
        .expect("append final prose");

        let timeline = snapshot("duplicate-card", &log);
        let chart_items: Vec<_> = timeline
            .items
            .iter()
            .filter(|item| {
                matches!(&item.content, OutputContent::Structured { output } if output.semantic_type == "chart")
            })
            .collect();
        assert_eq!(
            chart_items.len(),
            1,
            "the earlier attempt's card must be superseded, not left duplicated: {chart_items:?}"
        );
        assert_eq!(
            chart_items[0]
                .provenance
                .as_ref()
                .and_then(|p| p.tool_call_id.as_deref()),
            Some("call-2"),
            "the SURVIVING card must be the retried (latest) attempt, not the first"
        );
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
                input: serde_json::json!({"semantic_type":"chart","payload":{"chart_type":"line","series":[],"accessible_summary":"revenue"}}),
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
            &serde_json::json!({"semantic_type":"chart","payload":{"chart_type":"line","series":[],"accessible_summary":"revenue"}}),
        );
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::ToolUse {
                id: "call-2".into(),
                name: "emit_chart_card".into(),
                input: serde_json::json!({"semantic_type":"chart","payload":{"chart_type":"line","series":[],"accessible_summary":"cost"}}),
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
            &serde_json::json!({"semantic_type":"chart","payload":{"chart_type":"line","series":[],"accessible_summary":"cost"}}),
        );

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
            2,
            "two intentional cards of the same type with no repair nudge between them must both survive: {chart_items:?}"
        );
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
        let takeaways: Vec<_> = (0..12)
            .map(|i| serde_json::json!({"text": format!("Takeaway {i}: {}", "detail ".repeat(40)), "citation_indices": [1]}))
            .collect();
        let input = serde_json::json!({
            "semantic_type": "research.synthesis",
            "payload": {"sources": [{"title": "Reuters", "url": "https://example.com/a"}], "takeaways": takeaways}
        });
        assert!(
            input.to_string().len() > 3000,
            "fixture must exceed the limit"
        );
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::ToolUse {
                id: "call-r".into(),
                name: "emit_research_card".into(),
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
                    content: "Card displayed to the user (research.synthesis).".into(),
                    is_error: false,
                }],
            },
            meta: None,
        })
        .expect("result");
        append_presentation_for_call(&mut log, "emit_research_card", "call-r", &input);
        let timeline = snapshot("big-card", &log);
        assert!(
            timeline.items.iter().any(|item| matches!(&item.content,
                OutputContent::Structured { output } if output.semantic_type == "research.synthesis")),
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
            "semantic_type": "metric",
            "payload": {"label": "Temperature", "value": 25, "unit": "C"}
        });
        let call_entry = log
            .append_message(MessageRecord {
                message: Message::assistant(vec![ContentBlock::ToolUse {
                    id: "call-m".into(),
                    name: "emit_metric_card".into(),
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
        append_presentation_for_call(&mut log, "emit_metric_card", "call-m", &input);
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

    /// Not per-type and not per-size: every type the emit tools support, at a
    /// normal size and padded far past the tool framework's result line
    /// limit, must come out of the real `snapshot()` as exactly one card of
    /// that type — from the call arguments alone, with only an ack as result.
    #[test]
    fn every_supported_type_renders_through_the_full_path_at_any_size() {
        let cases = vak_core::presentation_tools::conformance_cases();
        assert!(
            cases.len() >= 97,
            "expected every registered type, got {}",
            cases.len()
        );
        let mut failures = Vec::new();
        for (tool, semantic_type, payload) in cases {
            for padded in [false, true] {
                let mut payload = payload.clone();
                if padded {
                    payload["x_padding"] = serde_json::Value::String("p".repeat(6000));
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
                let call_args =
                    serde_json::json!({"semantic_type": semantic_type, "payload": payload});
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

    #[test]
    fn every_built_in_pack_compiles_its_real_emitter_shape() {
        let cases = vak_core::presentation_tools::conformance_cases();
        let mut failures = Vec::new();
        for seed in vak_presentation::seeds::built_in_seed_pack() {
            let Some((_, semantic_type, payload)) = cases
                .iter()
                .find(|(_, kind, _)| seed.spec.accepts.iter().any(|accepted| accepted == kind))
            else {
                failures.push(format!("{} has no emitted semantic type", seed.spec.id));
                continue;
            };
            let compiled = vak_presentation::compile(
                &seed.spec,
                &vak_presentation::CompileInput {
                    semantic_type: (*semantic_type).into(),
                    payload: payload.clone(),
                    fallback_text: "Fallback".into(),
                },
            );
            if !matches!(compiled, vak_presentation::CompiledPresentation::Rich(_)) {
                failures.push(format!("{} ({semantic_type}): {compiled:?}", seed.spec.id));
            } else if vak_delivery::adaptive_presentation_markdown(&compiled)
                .trim()
                .is_empty()
            {
                failures.push(format!(
                    "{} ({semantic_type}): empty text delivery",
                    seed.spec.id
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "{} unusable packs:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    #[test]
    fn selected_metric_pack_keeps_every_reading_from_an_emitted_grid() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut log = channel_log(&dir, "metric-grid");
        log.append_message(MessageRecord {
            message: Message::user_text("How is the weather?"),
            meta: None,
        })
        .expect("user");
        let input = serde_json::json!({
            "semantic_type": "metric",
            "payload": {"label": "Noida now", "condition": "Sunny", "temperature": "35.2°C", "humidity": "31%"}
        });
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::ToolUse {
                id: "grid-call".into(),
                name: "emit_metric_card".into(),
                input: input.clone(),
            }]),
            meta: None,
        })
        .expect("call");
        log.append_message(MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "grid-call".into(),
                    content: "Card displayed to the user (metric).".into(),
                    is_error: false,
                }],
            },
            meta: None,
        })
        .expect("result");
        append_presentation_for_call(&mut log, "emit_metric_card", "grid-call", &input);
        let library = crate::effective_presentation_library(
            &vak_presentation::PresentationLibrary::default(),
            "/tmp/project",
        );
        let planner = vak_delivery::PresentationPlanner {
            skills: vak_delivery::built_in_skill_registry(),
            recipes: vak_delivery::built_in_recipes(),
        };
        let timeline =
            super::snapshot_with_planner_and_library("metric-grid", &log, &planner, &library);
        let card = timeline
            .items
            .iter()
            .find(|item| item.kind == OutputKind::Card)
            .expect("card");
        let OutputContent::Adaptive { tree, .. } = &card.content else {
            panic!("selected pack should produce adaptive content");
        };
        assert_eq!(tree.spec_id, "seed.metric");
        assert_eq!(
            tree.root.props.get("condition"),
            Some(&serde_json::json!("Sunny"))
        );
        assert_eq!(
            tree.root.props.get("temperature"),
            Some(&serde_json::json!("35.2°C"))
        );
        assert_eq!(
            tree.root.props.get("humidity"),
            Some(&serde_json::json!("31%"))
        );
        let lowered = vak_delivery::adaptive_presentation_markdown(
            &vak_presentation::CompiledPresentation::Rich(tree.clone()),
        );
        for reading in ["Sunny", "35.2°C", "31%"] {
            assert!(
                lowered.contains(reading),
                "constrained delivery lost {reading}: {lowered}"
            );
        }
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

    fn append_card_call(log: &mut SessionLog, id: &str, summary: &str) {
        let input = serde_json::json!({"semantic_type":"chart","payload":{"chart_type":"line","series":[],"accessible_summary":summary}});
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

    /// Channels deliver the card as the answer and retain only explicitly
    /// additional narration.
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
        assert!(!text.contains("The chart is shown above."), "{text}");
        let with_note = super::text_with_run_cards(&log, "Note: Check the holiday dip.".into());
        assert!(with_note.contains("sales rise steadily"), "{with_note}");
        assert!(with_note.ends_with("Check the holiday dip."), "{with_note}");
    }

    #[test]
    fn card_result_projects_only_an_explicit_additional_note() {
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
            OutputContent::Document { document } => document.metadata.get("card_note"),
            _ => None,
        });
        assert_eq!(
            note.map(String::as_str),
            Some("The holiday dip needs review.")
        );
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
    fn a_retried_card_is_delivered_once_and_a_nudge_is_not_a_turn() {
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
        let text = super::text_with_run_cards(&log, "(no text)".into());
        assert!(
            text.contains("RETRY") && !text.contains("FIRST-ATTEMPT"),
            "{text}"
        );
        assert!(
            !text.contains("(no text)"),
            "a card-only run has no placeholder: {text}"
        );
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
            "[stop-guard]: not done yet\nPlease continue.",
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
