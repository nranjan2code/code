//! Loss-accounting output engineering for channel delivery.
//!
//! The crate deliberately has no transport or server dependencies. It turns
//! a complete answer draft into a bounded, channel-specific packet in a
//! separate process. The source Markdown remains part of every packet so a
//! renderer can never become the system of record.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod adapters;
pub mod adaptive;
pub mod client;
pub mod discord;
pub mod outbox;
pub mod presentation;
pub mod skills;
pub mod slack;
pub mod telegram;
pub mod templates;

pub use adapters::{
    AdapterRegistry, ResultAdapter, built_in_adapters, structured_outputs_from_tool_result,
    structured_outputs_from_tool_result_with,
};
pub use adaptive::markdown as adaptive_presentation_markdown;
pub use adaptive::project as project_adaptive_presentation;
pub use adaptive::project_preferred as project_preferred_adaptive_presentation;
pub use presentation::{
    ArtifactRef, CalloutTone, Citation, DocumentBlock, DocumentCoverage,
    DocumentCoverageDisposition, InlineNode, OutputContent, OutputItem, OutputKind,
    OutputProvenance, OutputRole, OutputStatus, OutputStreamEvent, OutputStreamFrame,
    OutputTimeline, PRESENTATION_SCHEMA_VERSION, PresentationDocument, ResultOutcome,
    SurfaceCapabilities, TableAlignment, compile_markdown, inline_text, safe_link,
};
pub use skills::{
    ChartOutput, ChartPoint, ChartSeries, DecisionDisposition, LinkPreview, MediaOutput, Metric,
    PRESENTATION_SKILL_API, PlanDiagnostic, PresentationDecision, PresentationPlan,
    PresentationPlanner, PresentationRecipe, PresentationSkillManifest, RecipeCatalog,
    RendererBinding, RendererDecision, SignalContext, SkillError, SkillRegistry, StructuredOutput,
    built_in_recipes, built_in_skill_registry, link_previews_from_text, parse_fragment,
    parse_fragment_with, project_structured_fences, project_structured_fences_with,
    signals_from_context, signals_from_text, structured_markdown, structured_outputs_from_text,
    structured_outputs_from_text_with,
};
pub use vak_presentation as adaptive_presentation;

pub const DELIVERY_SCHEMA_VERSION: u16 = 2;

/// The complete answer and its conservative structural projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerDraft {
    pub schema_version: u16,
    pub source_markdown: String,
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub document: PresentationDocument,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    /// Independently presentable results. Empty means this draft is a single
    /// answer result represented by `source_markdown`.
    #[serde(default)]
    pub results: Vec<AnswerResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerResult {
    pub id: String,
    pub source_markdown: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ResultOutcome>,
}

impl AnswerDraft {
    /// Parse a useful initial block projection while retaining the exact
    /// source for export and fallback delivery.
    pub fn from_markdown(source_markdown: impl Into<String>) -> Self {
        let source_markdown = source_markdown.into();
        let blocks = parse_blocks(&source_markdown);
        let document = compile_markdown(source_markdown.clone());
        Self {
            schema_version: DELIVERY_SCHEMA_VERSION,
            source_markdown,
            blocks,
            document,
            metadata: BTreeMap::new(),
            results: Vec::new(),
        }
    }

    fn channel_projection(&self) -> Self {
        if self.results.is_empty() {
            return self.clone();
        }
        let source_markdown = self
            .results
            .iter()
            .map(|result| result.source_markdown.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut projected = Self::from_markdown(source_markdown);
        projected.metadata = self.metadata.clone();
        projected.results = self.results.clone();
        projected
    }
}

/// Stable, presentation-neutral blocks. New block types must retain a
/// textual fallback and receive a new schema version when their contract
/// changes incompatibly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Heading {
        id: String,
        level: u8,
        text: String,
    },
    Paragraph {
        id: String,
        text: String,
    },
    List {
        id: String,
        ordered: bool,
        items: Vec<String>,
    },
    Quote {
        id: String,
        text: String,
    },
    Code {
        id: String,
        language: Option<String>,
        content: String,
    },
    /// Unknown or not-yet-typed Markdown is intentionally opaque, not
    /// discarded. It can be rendered verbatim until a richer block exists.
    RawMarkdown {
        id: String,
        markdown: String,
        reason: String,
    },
}

impl Block {
    fn id(&self) -> &str {
        match self {
            Self::Heading { id, .. }
            | Self::Paragraph { id, .. }
            | Self::List { id, .. }
            | Self::Quote { id, .. }
            | Self::Code { id, .. }
            | Self::RawMarkdown { id, .. } => id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Markup {
    Plain,
    Markdown,
    TelegramHtml,
    SlackMrkdwn,
    DiscordMarkdown,
    Json,
}

/// Message classes are intentionally explicit. Only outward-facing content
/// may use a presentation template; system and control-plane messages keep
/// their own protocol shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryKind {
    Assistant,
    TaskSummary,
    Alert,
    User,
    Approval,
    Progress,
    ToolResult,
    System,
    Developer,
    ToolCall,
    Steering,
    Internal,
}

impl DeliveryKind {
    fn allows_template(self) -> bool {
        matches!(
            self,
            Self::Assistant | Self::TaskSummary | Self::Alert | Self::User
        )
    }

    fn is_external(self) -> bool {
        !matches!(self, Self::System | Self::Developer | Self::Internal)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeliveryContent {
    Answer(AnswerDraft),
    Approval(ApprovalPayload),
    Progress(ProgressPayload),
    ToolResult(ToolResultPayload),
    Text { markdown: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalPayload {
    pub request_id: String,
    pub title: String,
    pub detail: String,
    pub expires_at: Option<String>,
    pub actions: Vec<DeliveryAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressPayload {
    pub label: String,
    pub state: String,
    pub percent: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResultPayload {
    pub tool: String,
    pub output: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryAction {
    pub id: String,
    pub label: String,
    pub verb: String,
    pub data: BTreeMap<String, String>,
}

/// What a target can safely accept. Profiles are input to rendering, not
/// claims inferred from a target name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryProfile {
    pub surface: String,
    pub markup: Markup,
    pub max_chars: Option<usize>,
    pub supports_tables: bool,
    pub supports_code_blocks: bool,
    pub supports_links: bool,
    pub supports_actions: bool,
    /// Optional declarative layout. `None` means the channel default.
    #[serde(default)]
    pub template: Option<TemplateSpec>,
    /// When this packet should reach its reader and how hard it may push.
    /// The posture is set by the caller (session/task config); it decides
    /// whether a packet goes out now, waits for completion, or rolls into
    /// a digest. It never changes the content or the renderer.
    #[serde(default)]
    pub posture: DeliveryPosture,
}

impl DeliveryProfile {
    pub fn plain(surface: impl Into<String>) -> Self {
        Self {
            surface: surface.into(),
            markup: Markup::Plain,
            max_chars: None,
            supports_tables: false,
            supports_code_blocks: true,
            supports_links: true,
            supports_actions: false,
            template: None,
            posture: DeliveryPosture::default(),
        }
    }

    fn validate(&self) -> Result<(), DeliveryError> {
        if self.surface.trim().is_empty() {
            return Err(DeliveryError::InvalidProfile("surface is empty".into()));
        }
        if self.max_chars == Some(0) {
            return Err(DeliveryError::InvalidProfile(
                "max_chars must be greater than zero".into(),
            ));
        }
        Ok(())
    }

    /// Semantic capabilities consumed by native and constrained projectors.
    /// Existing profile fields remain the compatibility wire contract.
    pub fn capabilities(&self) -> SurfaceCapabilities {
        let native = matches!(self.surface.as_str(), "desktop" | "tui" | "admin");
        SurfaceCapabilities {
            structured_blocks: matches!(self.markup, Markup::Json) || native,
            tables: self.supports_tables,
            code: self.supports_code_blocks,
            links: self.supports_links,
            media: native || matches!(self.markup, Markup::Json),
            file_references: native || matches!(self.markup, Markup::Json),
            actions: self.supports_actions,
            color: native,
            interactive: self.supports_actions || native,
            accessible_plain: matches!(self.markup, Markup::Plain),
            max_chars: self.max_chars,
        }
    }
}

/// User-replaceable presentation layout. Templates contain no executable
/// code, network access, filesystem access, or arbitrary markup injection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateSpec {
    pub id: String,
    pub revision: u32,
    pub origin: TemplateOrigin,
    #[serde(default)]
    pub activation: TemplateActivation,
    #[serde(default)]
    pub nodes: Vec<TemplateNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateOrigin {
    BuiltIn,
    User,
    Project,
    AgentProposal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateActivation {
    #[default]
    Active,
    Proposed,
}

/// Resolves replaceable templates without allowing an agent proposal to
/// silently become active. User/project templates override built-ins by
/// origin priority, then by revision.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateRegistry {
    pub templates: Vec<TemplateSpec>,
}

impl TemplateRegistry {
    pub fn upsert(&mut self, template: TemplateSpec) -> Result<(), DeliveryError> {
        template.validate()?;
        if let Some(existing) = self
            .templates
            .iter_mut()
            .find(|candidate| candidate.id == template.id && candidate.origin == template.origin)
        {
            if template.revision < existing.revision {
                return Err(DeliveryError::InvalidTemplate(format!(
                    "template {} revision {} is older than stored revision {}",
                    template.id, template.revision, existing.revision
                )));
            }
            *existing = template;
        } else {
            self.templates.push(template);
        }
        Ok(())
    }

    pub fn resolve(&self, id: &str) -> Option<&TemplateSpec> {
        self.templates
            .iter()
            .filter(|template| {
                template.id == id && template.activation == TemplateActivation::Active
            })
            .max_by_key(|template| (origin_priority(template.origin), template.revision))
    }

    pub fn activate_proposal(&mut self, id: &str, revision: u32) -> bool {
        self.templates.iter_mut().any(|template| {
            if template.id == id
                && template.revision == revision
                && template.origin == TemplateOrigin::AgentProposal
            {
                template.activation = TemplateActivation::Active;
                true
            } else {
                false
            }
        })
    }
}

fn origin_priority(origin: TemplateOrigin) -> u8 {
    match origin {
        TemplateOrigin::BuiltIn => 1,
        TemplateOrigin::AgentProposal => 2,
        TemplateOrigin::Project => 3,
        TemplateOrigin::User => 4,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TemplateNode {
    Literal {
        text: String,
    },
    Slot {
        slot: TemplateSlot,
    },
    IfPresent {
        slot: TemplateSlot,
        then_nodes: Vec<TemplateNode>,
        else_nodes: Vec<TemplateNode>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateSlot {
    Title,
    Body,
    SourceMarkdown,
    BlockCount,
    Metadata { key: String },
}

impl TemplateSpec {
    pub fn validate(&self) -> Result<(), DeliveryError> {
        if self.id.trim().is_empty() {
            return Err(DeliveryError::InvalidTemplate("id is empty".into()));
        }
        if self.nodes.len() > 128 {
            return Err(DeliveryError::InvalidTemplate(
                "template has more than 128 top-level nodes".into(),
            ));
        }
        validate_nodes(&self.nodes, 0)
    }

    fn render(
        &self,
        answer: &AnswerDraft,
        project_structured: bool,
        skills: &SkillRegistry,
    ) -> Result<String, DeliveryError> {
        self.validate()?;
        render_nodes(&self.nodes, answer, 0, project_structured, skills)
    }
}

fn validate_nodes(nodes: &[TemplateNode], depth: usize) -> Result<(), DeliveryError> {
    if depth > 8 {
        return Err(DeliveryError::InvalidTemplate(
            "template nesting exceeds 8 levels".into(),
        ));
    }
    for node in nodes {
        match node {
            TemplateNode::Literal { text } if text.chars().count() > 16_384 => {
                return Err(DeliveryError::InvalidTemplate(
                    "template literal exceeds 16384 characters".into(),
                ));
            }
            TemplateNode::Literal { .. } | TemplateNode::Slot { .. } => {}
            TemplateNode::IfPresent {
                then_nodes,
                else_nodes,
                ..
            } => {
                validate_nodes(then_nodes, depth + 1)?;
                validate_nodes(else_nodes, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn render_nodes(
    nodes: &[TemplateNode],
    answer: &AnswerDraft,
    depth: usize,
    project_structured: bool,
    skills: &SkillRegistry,
) -> Result<String, DeliveryError> {
    if depth > 8 {
        return Err(DeliveryError::InvalidTemplate(
            "template nesting exceeds 8 levels".into(),
        ));
    }
    let mut out = String::new();
    for node in nodes {
        match node {
            TemplateNode::Literal { text } => out.push_str(text),
            TemplateNode::Slot { slot } => {
                out.push_str(&slot_value(slot, answer, project_structured, skills))
            }
            TemplateNode::IfPresent {
                slot,
                then_nodes,
                else_nodes,
            } => {
                let selected = if slot_value(slot, answer, project_structured, skills).is_empty() {
                    else_nodes
                } else {
                    then_nodes
                };
                out.push_str(&render_nodes(
                    selected,
                    answer,
                    depth + 1,
                    project_structured,
                    skills,
                )?);
            }
        }
    }
    Ok(out)
}

fn slot_value(
    slot: &TemplateSlot,
    answer: &AnswerDraft,
    project_structured: bool,
    skills: &SkillRegistry,
) -> String {
    match slot {
        TemplateSlot::Title => answer
            .blocks
            .iter()
            .find_map(|block| match block {
                Block::Heading { text, .. } => Some(text.clone()),
                _ => None,
            })
            .or_else(|| answer.metadata.get("title").cloned())
            .unwrap_or_default(),
        TemplateSlot::Body => render_plain(answer, project_structured, skills),
        TemplateSlot::SourceMarkdown => answer.source_markdown.clone(),
        TemplateSlot::BlockCount => answer.blocks.len().to_string(),
        TemplateSlot::Metadata { key } => answer.metadata.get(key).cloned().unwrap_or_default(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryJob {
    pub job_id: String,
    pub target: String,
    pub kind: DeliveryKind,
    pub content: DeliveryContent,
    pub profile: DeliveryProfile,
    /// Plugin-merged skill registry for structured-fence validation. When
    /// `None`, the worker falls back to `built_in_skill_registry()`. The Core
    /// populates this from its capability snapshot so that plugin-declared
    /// semantic types are recognized during rendering without the worker
    /// having filesystem or network access.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_registry: Option<SkillRegistry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryPacket {
    pub schema_version: u16,
    pub job_id: String,
    pub target: String,
    pub surface: String,
    pub kind: DeliveryKind,
    pub payload: DeliveryPayload,
    /// Exact source is always available for export, audit, and fallback.
    pub fallback_markdown: String,
    pub chunks: Vec<String>,
    pub actions: Vec<DeliveryAction>,
    pub coverage: Vec<Coverage>,
    pub diagnostics: Vec<String>,
    /// Native semantic projection. Text-only consumers can ignore this and
    /// continue using `chunks` or `fallback_markdown`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<OutputTimeline>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum DeliveryPayload {
    Text(String),
    Structured(DeliveryContent),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    pub block_id: String,
    pub disposition: CoverageDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoverageDisposition {
    Rendered,
    PreservedInFallback { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryError {
    InvalidProfile(String),
    InvalidTemplate(String),
    InvalidMessage(String),
    UnsupportedSchema(u16),
}

impl std::fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidProfile(message) => write!(f, "invalid delivery profile: {message}"),
            Self::InvalidTemplate(message) => write!(f, "invalid delivery template: {message}"),
            Self::InvalidMessage(message) => write!(f, "invalid delivery message: {message}"),
            Self::UnsupportedSchema(version) => {
                write!(f, "unsupported delivery schema version: {version}")
            }
        }
    }
}

impl std::error::Error for DeliveryError {}

pub fn render(job: &DeliveryJob) -> Result<DeliveryPacket, DeliveryError> {
    job.profile.validate()?;
    if !job.kind.is_external() {
        return Err(DeliveryError::InvalidMessage(format!(
            "{} messages are control-plane data and must use the event stream",
            format_kind(job.kind)
        )));
    }

    if !content_matches_kind(job.kind, &job.content) {
        return Err(DeliveryError::InvalidMessage(
            "message kind and content type do not match".into(),
        ));
    }
    if let DeliveryContent::Answer(answer) = &job.content
        && answer.schema_version != DELIVERY_SCHEMA_VERSION
    {
        return Err(DeliveryError::UnsupportedSchema(answer.schema_version));
    }
    if job.profile.template.is_some() && !job.kind.allows_template() {
        return Err(DeliveryError::InvalidTemplate(format!(
            "templates are not allowed for {} messages",
            format_kind(job.kind)
        )));
    }
    if let Some(template) = job.profile.template.as_ref()
        && template.activation != TemplateActivation::Active
    {
        return Err(DeliveryError::InvalidTemplate(
            "agent template proposals require explicit activation".into(),
        ));
    }

    let (rendered, fallback_markdown, actions, coverage) = render_content(job)?;
    let chunks = match job.profile.markup {
        Markup::TelegramHtml => telegram::split_html_chunks(&rendered, job.profile.max_chars),
        Markup::SlackMrkdwn | Markup::DiscordMarkdown => {
            chunk_markdown_preserving_fences(&rendered, job.profile.max_chars)
        }
        _ => chunk_text(&rendered, job.profile.max_chars),
    };
    let payload = match job.profile.markup {
        Markup::Json => DeliveryPayload::Structured(job.content.clone()),
        _ => DeliveryPayload::Text(rendered),
    };
    let mut diagnostics = Vec::new();
    if !actions.is_empty() && !job.profile.supports_actions {
        diagnostics.push(
            "actions preserved separately; target does not declare interactive action support"
                .into(),
        );
    }

    let presentation = presentation_for_job(job);
    Ok(DeliveryPacket {
        schema_version: DELIVERY_SCHEMA_VERSION,
        job_id: job.job_id.clone(),
        target: job.target.clone(),
        surface: job.profile.surface.clone(),
        kind: job.kind,
        payload,
        fallback_markdown,
        chunks,
        actions,
        coverage,
        diagnostics,
        presentation,
    })
}

fn presentation_for_job(job: &DeliveryJob) -> Option<OutputTimeline> {
    let DeliveryContent::Answer(answer) = &job.content else {
        return None;
    };
    let default_status = match (
        answer
            .metadata
            .get("outcome_completion")
            .map(String::as_str),
        answer
            .metadata
            .get("outcome_evidence_state")
            .map(String::as_str),
    ) {
        (Some("complete"), None | Some("fresh")) | (None, None) => OutputStatus::Succeeded,
        _ => OutputStatus::Partial,
    };
    let default_status = if answer
        .metadata
        .get("outcome_human_review")
        .is_some_and(|value| !value.trim().is_empty() && value != "not_required")
    {
        OutputStatus::Partial
    } else {
        default_status
    };
    let legacy_document = answer.document.clone();
    let has_typed_results = !answer.results.is_empty();
    let results = if answer.results.is_empty() {
        vec![AnswerResult {
            id: "answer".into(),
            source_markdown: answer.source_markdown.clone(),
            outcome: metadata_outcome(&answer.metadata, default_status),
        }]
    } else {
        answer.results.clone()
    };
    let mut diagnostics = Vec::new();
    let items = results
        .into_iter()
        .map(|result| {
            let outcome = result.outcome;
            let status = outcome.as_ref().map(|value| value.status).unwrap_or(default_status);
            if status == OutputStatus::Partial {
                diagnostics.push(format!(
                    "result '{}' is incomplete or unverified; inspect evidence before relying on it",
                    result.id
                ));
            }
            OutputItem {
                id: format!("{}/{}", job.job_id, result.id),
                timestamp: String::new(),
                turn_id: job.job_id.clone(),
                role: OutputRole::Assistant,
                kind: match job.kind {
                    DeliveryKind::Assistant | DeliveryKind::TaskSummary | DeliveryKind::Alert => OutputKind::Outcome,
                    _ => OutputKind::Message,
                },
                status,
                outcome,
                content: OutputContent::Document {
                    document: if !has_typed_results
                        && result.id == "answer"
                        && legacy_document.source_markdown == result.source_markdown
                    {
                        legacy_document.clone()
                    } else {
                        compile_markdown(result.source_markdown.clone())
                    },
                },
                provenance: None,
                actions: Vec::new(),
                fallback_text: result.source_markdown,
            }
        })
        .collect();
    Some(OutputTimeline {
        schema_version: presentation::PRESENTATION_SCHEMA_VERSION,
        session_id: String::new(),
        cursor: None,
        items,
        diagnostics,
        goal: None,
    })
}

fn metadata_outcome(
    metadata: &BTreeMap<String, String>,
    status: OutputStatus,
) -> Option<ResultOutcome> {
    let completion = metadata.get("outcome_completion").cloned();
    let evidence_state = metadata.get("outcome_evidence_state").cloned();
    let human_review = metadata.get("outcome_human_review").cloned();
    (completion.is_some() || evidence_state.is_some() || human_review.is_some()).then(|| {
        ResultOutcome {
            result_id: "answer".into(),
            status,
            completion,
            evidence_state,
            requirement_ids: Vec::new(),
            evidence_receipt_ids: Vec::new(),
            evidence: Vec::new(),
            human_review,
        }
    })
}

/// Whether the markup type cannot render semantic AST blocks natively and
/// therefore needs ```vak fences projected to readable text before markup
/// conversion. Native surfaces (desktop, tui, admin) and JSON payloads handle
/// structured blocks through the semantic path; all chat and plain-text
/// surfaces need projection.
fn needs_structured_projection(markup: Markup) -> bool {
    !matches!(markup, Markup::Json)
}

fn render_content(
    job: &DeliveryJob,
) -> Result<(String, String, Vec<DeliveryAction>, Vec<Coverage>), DeliveryError> {
    let project_structured = needs_structured_projection(job.profile.markup);
    let skills = job
        .skill_registry
        .as_ref()
        .map(std::borrow::Cow::Borrowed)
        .unwrap_or_else(|| std::borrow::Cow::Owned(built_in_skill_registry()));
    let skills_ref = &*skills;
    match &job.content {
        DeliveryContent::Answer(answer) => {
            let projected_answer = answer.channel_projection();
            let rendered = match job.profile.template.as_ref() {
                Some(template) => {
                    let text =
                        template.render(&projected_answer, project_structured, skills_ref)?;
                    let text = if project_structured {
                        project_structured_fences_with(&text, skills_ref)
                    } else {
                        text
                    };
                    render_text(&text, job.profile.markup)
                }
                None => render_answer(
                    &projected_answer,
                    job.profile.markup,
                    project_structured,
                    skills_ref,
                ),
            };
            let coverage = projected_answer
                .blocks
                .iter()
                .map(|block| Coverage {
                    block_id: block.id().to_string(),
                    disposition: CoverageDisposition::Rendered,
                })
                .collect();
            Ok((
                rendered,
                projected_answer.source_markdown,
                Vec::new(),
                coverage,
            ))
        }
        DeliveryContent::Approval(approval) => Ok((
            render_approval(approval, job.profile.markup),
            approval.detail.clone(),
            approval.actions.clone(),
            Vec::new(),
        )),
        DeliveryContent::Progress(progress) => Ok((
            render_progress(progress, job.profile.markup),
            format!("{}: {}", progress.label, progress.state),
            Vec::new(),
            Vec::new(),
        )),
        DeliveryContent::ToolResult(result) => Ok((
            render_tool_result(result, job.profile.markup, project_structured, skills_ref),
            result.output.clone(),
            Vec::new(),
            Vec::new(),
        )),
        DeliveryContent::Text { markdown } => {
            let projected = if project_structured {
                project_structured_fences_with(markdown, skills_ref)
            } else {
                markdown.clone()
            };
            Ok((
                render_text(&projected, job.profile.markup),
                markdown.clone(),
                Vec::new(),
                Vec::new(),
            ))
        }
    }
}

fn content_matches_kind(kind: DeliveryKind, content: &DeliveryContent) -> bool {
    matches!(
        (kind, content),
        (
            DeliveryKind::Assistant | DeliveryKind::TaskSummary | DeliveryKind::Alert,
            DeliveryContent::Answer(_)
        ) | (DeliveryKind::User, DeliveryContent::Answer(_))
            | (DeliveryKind::Approval, DeliveryContent::Approval(_))
            | (DeliveryKind::Progress, DeliveryContent::Progress(_))
            | (DeliveryKind::ToolResult, DeliveryContent::ToolResult(_))
            | (_, DeliveryContent::Text { .. })
    )
}

fn format_kind(kind: DeliveryKind) -> &'static str {
    match kind {
        DeliveryKind::Assistant => "assistant",
        DeliveryKind::TaskSummary => "task_summary",
        DeliveryKind::Alert => "alert",
        DeliveryKind::User => "user",
        DeliveryKind::Approval => "approval",
        DeliveryKind::Progress => "progress",
        DeliveryKind::ToolResult => "tool_result",
        DeliveryKind::System => "system",
        DeliveryKind::Developer => "developer",
        DeliveryKind::ToolCall => "tool_call",
        DeliveryKind::Steering => "steering",
        DeliveryKind::Internal => "internal",
    }
}

fn is_scaffolding_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("Surface:")
        || trimmed.starts_with("Outcome:")
        || trimmed.starts_with("primary deliverable:")
        || trimmed.eq_ignore_ascii_case("completed")
        || trimmed.starts_with("contract_id:")
        || trimmed.eq_ignore_ascii_case("vak")
        || trimmed.starts_with("[stop-guard]")
        || trimmed.starts_with("[stop-hook]")
        || trimmed.starts_with("I will write and execute this within the sandbox")
}

fn strip_control_blocks(text: &str) -> String {
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

    for prefix in ["[stop-guard]:", "[stop-hook]:"] {
        while let Some(start) = out.find(prefix) {
            let remainder = &out[start..];
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

pub fn clean_scaffolding(text: &str) -> String {
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

fn render_answer(
    answer: &AnswerDraft,
    markup: Markup,
    project_structured: bool,
    skills: &SkillRegistry,
) -> String {
    let metadata_notice = match (
        answer
            .metadata
            .get("outcome_completion")
            .map(String::as_str),
        answer
            .metadata
            .get("outcome_evidence_state")
            .map(String::as_str),
    ) {
        (Some("complete"), None | Some("fresh")) | (None, None) => None,
        _ => Some(
            "⚠️ Outcome incomplete or unverified — review the evidence before relying on this result.\n\n"
                .into(),
        ),
    };
    let result_notice = answer.results.iter().find_map(|result| {
        let outcome = result.outcome.as_ref()?;
        (outcome.status != OutputStatus::Succeeded
            || outcome.evidence_state.as_deref().is_some_and(|state| state != "fresh")
            || outcome.human_review.is_some())
        .then_some(format!(
            "⚠️ Result '{}' is incomplete, unverified, or requires review — inspect its evidence before relying on it.\n\n",
            result.id
        ))
    });
    let notice = result_notice.or(metadata_notice);
    let raw_source = if project_structured {
        project_structured_fences_with(&answer.source_markdown, skills)
    } else {
        answer.source_markdown.clone()
    };
    let source = clean_scaffolding(&raw_source);
    match markup {
        Markup::Plain => format!(
            "{}{}",
            notice.unwrap_or_default(),
            render_plain(answer, project_structured, skills)
        ),
        Markup::Markdown => format!("{}{}", notice.unwrap_or_default(), source),
        Markup::TelegramHtml => {
            telegram::markdown_to_html(&format!("{}{}", notice.unwrap_or_default(), source))
        }
        Markup::SlackMrkdwn => {
            slack::markdown_to_mrkdwn(&format!("{}{}", notice.unwrap_or_default(), source))
        }
        Markup::DiscordMarkdown => {
            discord::markdown_to_discord(&format!("{}{}", notice.unwrap_or_default(), source))
        }
        Markup::Json => String::new(),
    }
}

/// Markup conversion only — does NOT project structured fences. Callers must
/// project first via `project_structured_fences` when the surface doesn't
/// support structured blocks natively.
fn render_text(markdown: &str, markup: Markup) -> String {
    match markup {
        Markup::TelegramHtml => telegram::markdown_to_html(markdown),
        Markup::SlackMrkdwn => slack::markdown_to_mrkdwn(markdown),
        Markup::DiscordMarkdown => discord::markdown_to_discord(markdown),
        Markup::Plain | Markup::Markdown | Markup::Json => markdown.to_string(),
    }
}

fn render_progress(progress: &ProgressPayload, markup: Markup) -> String {
    let text = format!("{}: {}", progress.label, progress.state);
    if matches!(markup, Markup::TelegramHtml) {
        escape_html(&text)
    } else {
        text
    }
}

fn render_tool_result(
    result: &ToolResultPayload,
    markup: Markup,
    project_structured: bool,
    skills: &SkillRegistry,
) -> String {
    let prefix = if result.is_error {
        "Tool error"
    } else {
        "Tool result"
    };
    let formatted = format!("{prefix} ({}\n\n{}", result.tool, result.output);
    let text = if project_structured {
        project_structured_fences_with(&formatted, skills)
    } else {
        formatted
    };
    if matches!(markup, Markup::TelegramHtml) {
        escape_html(&text)
    } else {
        text
    }
}

fn render_approval(approval: &ApprovalPayload, markup: Markup) -> String {
    let mut text = format!("{}\n\n{}", approval.title, approval.detail);
    if let Some(expires_at) = &approval.expires_at {
        text.push_str(&format!("\n\nExpires: {expires_at}"));
    }
    if !approval.actions.is_empty() {
        text.push_str("\n\nActions: ");
        text.push_str(
            &approval
                .actions
                .iter()
                .map(|action| action.label.as_str())
                .collect::<Vec<_>>()
                .join(" / "),
        );
    }
    if matches!(markup, Markup::TelegramHtml) {
        escape_html(&text)
    } else {
        text
    }
}

fn parse_blocks(source: &str) -> Vec<Block> {
    let lines: Vec<&str> = source.lines().collect();
    let mut blocks = Vec::new();
    let mut index = 0;
    let mut block_number = 0;

    while index < lines.len() {
        if lines[index].trim().is_empty() {
            index += 1;
            continue;
        }
        let id = || format!("block-{block_number}");
        let line = lines[index];
        let trimmed = line.trim_start();

        if let Some(fence) = trimmed.strip_prefix("```") {
            let language = (!fence.trim().is_empty()).then(|| fence.trim().to_string());
            let start = index + 1;
            index = start;
            while index < lines.len() && !lines[index].trim_start().starts_with("```") {
                index += 1;
            }
            blocks.push(Block::Code {
                id: id(),
                language,
                content: lines[start..index].join("\n"),
            });
            block_number += 1;
            index += usize::from(index < lines.len());
            continue;
        }

        if let Some((level, text)) = heading(line) {
            blocks.push(Block::Heading {
                id: id(),
                level,
                text: text.to_string(),
            });
            block_number += 1;
            index += 1;
            continue;
        }

        if let Some((ordered, item)) = list_item(line) {
            let mut items = vec![item.to_string()];
            index += 1;
            while index < lines.len() {
                let Some((same_order, next)) = list_item(lines[index]) else {
                    break;
                };
                if same_order != ordered {
                    break;
                }
                items.push(next.to_string());
                index += 1;
            }
            blocks.push(Block::List {
                id: id(),
                ordered,
                items,
            });
            block_number += 1;
            continue;
        }

        if let Some(text) = trimmed.strip_prefix("> ") {
            blocks.push(Block::Quote {
                id: id(),
                text: text.to_string(),
            });
            block_number += 1;
            index += 1;
            continue;
        }

        if trimmed.starts_with('|') && trimmed.ends_with('|') {
            let start = index;
            index += 1;
            while index < lines.len()
                && lines[index].trim_start().starts_with('|')
                && lines[index].trim_end().ends_with('|')
            {
                index += 1;
            }
            blocks.push(Block::RawMarkdown {
                id: id(),
                markdown: lines[start..index].join("\n"),
                reason: "table semantics require a channel capability".into(),
            });
            block_number += 1;
            continue;
        }

        let start = index;
        index += 1;
        while index < lines.len()
            && !lines[index].trim().is_empty()
            && heading(lines[index]).is_none()
            && list_item(lines[index]).is_none()
            && !lines[index].trim_start().starts_with("> ")
            && !lines[index].trim_start().starts_with("```")
        {
            index += 1;
        }
        blocks.push(Block::Paragraph {
            id: id(),
            text: lines[start..index].join("\n"),
        });
        block_number += 1;
    }
    blocks
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let trimmed = line.trim_start();
    let count = trimmed.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&count) && trimmed.as_bytes().get(count) == Some(&b' ') {
        Some((count as u8, trimmed[count + 1..].trim()))
    } else {
        None
    }
}

fn list_item(line: &str) -> Option<(bool, &str)> {
    let trimmed = line.trim_start();
    if let Some(item) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        return Some((false, item));
    }
    let digit_count = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_count > 0 && trimmed.as_bytes().get(digit_count..digit_count + 2) == Some(b". ") {
        return Some((true, &trimmed[digit_count + 2..]));
    }
    None
}

fn render_plain(answer: &AnswerDraft, project_structured: bool, skills: &SkillRegistry) -> String {
    answer
        .blocks
        .iter()
        .map(|block| match block {
            Block::Heading { text, .. } => text.clone(),
            Block::Paragraph { text, .. } => text.clone(),
            Block::List { ordered, items, .. } => items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    if *ordered {
                        format!("{}. {item}", index + 1)
                    } else {
                        format!("- {item}")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Block::Quote { text, .. } => format!("> {text}"),
            Block::Code {
                content, language, ..
            } if project_structured && language.as_deref() == Some("vak") => {
                parse_fragment_with(content, skills)
                    .map(|output| structured_markdown(&output))
                    .unwrap_or_else(|_| content.clone())
            }
            Block::Code { content, .. }
            | Block::RawMarkdown {
                markdown: content, ..
            } => content.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn chunk_text(text: &str, max_chars: Option<usize>) -> Vec<String> {
    let Some(max_chars) = max_chars else {
        return vec![text.to_string()];
    };
    if text.is_empty() {
        return vec![String::new()];
    }
    let chars: Vec<char> = text.chars().collect();
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let hard_end = (start + max_chars).min(chars.len());
        let end = chars[start..hard_end]
            .iter()
            .rposition(|c| *c == '\n')
            .map(|offset| start + offset)
            .filter(|candidate| *candidate > start)
            .unwrap_or(hard_end);
        chunks.push(chars[start..end].iter().collect());
        start = end;
        while start < chars.len() && chars[start] == '\n' {
            start += 1;
        }
    }
    chunks
}

/// Chunk a Slack/Discord `mrkdwn`-ish message on line boundaries like
/// [`chunk_text`], but close and reopen a fenced code block (```` ``` ````)
/// at any boundary that falls inside one, so a split never leaves a
/// dangling fence that swallows the rest of the message as code.
fn chunk_markdown_preserving_fences(text: &str, max_chars: Option<usize>) -> Vec<String> {
    let Some(max_chars) = max_chars else {
        return vec![text.to_string()];
    };
    if text.chars().count() <= max_chars {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut in_fence = false;
    let mut fence_header = String::new();

    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start().trim_end_matches('\n');
        let is_fence_marker = trimmed.starts_with("```");

        let reserve = if in_fence { 4 } else { 0 }; // room for a closing "```\n"
        if !current.is_empty()
            && current.chars().count() + line.chars().count() + reserve > max_chars
        {
            if in_fence {
                current.push_str("```\n");
            }
            chunks.push(std::mem::take(&mut current));
            if in_fence {
                current.push_str(&fence_header);
                current.push('\n');
            }
        }
        current.push_str(line);
        if is_fence_marker {
            if !in_fence {
                fence_header = trimmed.to_string();
            }
            in_fence = !in_fence;
        }
    }
    if !current.trim().is_empty() {
        chunks.push(current);
    }
    chunks
}

pub mod worker {
    //! Line-oriented worker protocol used by the future server outbox.

    use super::{DeliveryJob, DeliveryPacket, render};
    use serde::{Deserialize, Serialize};
    use std::io::{self, BufRead, Write};

    pub const WORKER_PROTOCOL_VERSION: u16 = 1;
    pub const WORKER_SUBCOMMAND: &str = "__delivery_worker";

    #[derive(Debug, Serialize, Deserialize)]
    pub struct WorkerRequest {
        pub protocol_version: u16,
        pub job: DeliveryJob,
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct WorkerResponse {
        pub protocol_version: u16,
        pub job_id: Option<String>,
        pub packet: Option<DeliveryPacket>,
        pub error: Option<String>,
    }

    pub fn process_line(line: &str) -> String {
        let parsed = serde_json::from_str::<WorkerRequest>(line);
        let response = match parsed {
            Ok(request) if request.protocol_version == WORKER_PROTOCOL_VERSION => {
                let job_id = request.job.job_id.clone();
                match render(&request.job) {
                    Ok(packet) => WorkerResponse {
                        protocol_version: WORKER_PROTOCOL_VERSION,
                        job_id: Some(job_id),
                        packet: Some(packet),
                        error: None,
                    },
                    Err(error) => WorkerResponse {
                        protocol_version: WORKER_PROTOCOL_VERSION,
                        job_id: Some(job_id),
                        packet: None,
                        error: Some(error.to_string()),
                    },
                }
            }
            Ok(request) => WorkerResponse {
                protocol_version: WORKER_PROTOCOL_VERSION,
                job_id: Some(request.job.job_id),
                packet: None,
                error: Some(format!(
                    "unsupported worker protocol version {}",
                    request.protocol_version
                )),
            },
            Err(error) => WorkerResponse {
                protocol_version: WORKER_PROTOCOL_VERSION,
                job_id: None,
                packet: None,
                error: Some(format!("invalid delivery worker request: {error}")),
            },
        };
        serde_json::to_string(&response).unwrap_or_else(|error| {
            format!(
                "{{\"protocol_version\":{WORKER_PROTOCOL_VERSION},\"error\":\"response serialization failed: {error}\"}}"
            )
        })
    }

    pub fn run_stdio() -> i32 {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut output = stdout.lock();
        for line in stdin.lock().lines() {
            match line {
                Ok(line) => {
                    if writeln!(output, "{}", process_line(&line)).is_err() {
                        return 1;
                    }
                }
                Err(_) => return 1,
            }
        }
        0
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn job(markup: Markup, max_chars: Option<usize>) -> DeliveryJob {
        let source =
            "# Result\n\nDone.\n\n```rust\nlet x = 1;\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        DeliveryJob {
            job_id: "job-1".into(),
            target: "test:one".into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(AnswerDraft::from_markdown(source)),
            profile: DeliveryProfile {
                surface: "test".into(),
                markup,
                max_chars,
                supports_tables: false,
                supports_code_blocks: true,
                supports_links: true,
                supports_actions: false,
                template: None,
                posture: DeliveryPosture::default(),
            },
            skill_registry: None,
        }
    }

    #[test]
    fn source_and_unknown_blocks_are_preserved() {
        let DeliveryContent::Answer(answer) = &job(Markup::Plain, None).content else {
            panic!("test job must contain an answer");
        };
        assert!(answer.source_markdown.contains("| a | b |"));
        assert!(
            answer
                .blocks
                .iter()
                .any(|block| matches!(block, Block::RawMarkdown { .. }))
        );
    }

    #[test]
    fn packet_keeps_exact_fallback_and_coverage() {
        let input = job(Markup::TelegramHtml, Some(4096));
        let packet = render(&input).expect("valid delivery job");
        let DeliveryContent::Answer(answer) = &input.content else {
            panic!("test job must contain an answer");
        };
        assert_eq!(packet.fallback_markdown, answer.source_markdown);
        assert_eq!(packet.coverage.len(), answer.blocks.len());
        assert!(
            packet
                .chunks
                .iter()
                .all(|chunk| chunk.chars().count() <= 4096)
        );
        assert!(matches!(packet.payload, DeliveryPayload::Text(_)));
        let presentation = packet
            .presentation
            .expect("schema v2 packet has a timeline");
        assert_eq!(presentation.items.len(), 1);
        let OutputContent::Document { document } = &presentation.items[0].content else {
            panic!("answer must project to a document");
        };
        assert_eq!(document.source_markdown, answer.source_markdown);
        assert_eq!(document.coverage.len(), document.blocks.len());
        assert!(
            document
                .blocks
                .iter()
                .any(|block| matches!(block, DocumentBlock::Table { .. }))
        );
    }

    #[test]
    fn specialized_structured_outputs_keep_one_lossless_fallback_across_surfaces() {
        // These are deliberately generic semantic shapes: the delivery layer may
        // lower them for a constrained channel, but it must never replace the
        // source answer that replay/export/voice consumers depend on.
        let source = concat!(
            "# Weekly review\n\n",
            "```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"items\",\"value\":12}}\n```\n\n",
            "```vak\n{\"semantic_type\":\"collection\",\"payload\":{\"title\":\"Next\",\"items\":[{\"label\":\"One\"}]}}\n```\n\n",
            "```vak\n{\"semantic_type\":\"steps\",\"payload\":{\"title\":\"Plan\",\"steps\":[{\"title\":\"Review\"}]}}\n```\n",
        );
        for (surface, markup) in [
            ("telegram", Markup::TelegramHtml),
            ("slack", Markup::SlackMrkdwn),
            ("discord", Markup::DiscordMarkdown),
            ("voice", Markup::Plain),
            ("webhook", Markup::Json),
        ] {
            let input = DeliveryJob {
                job_id: format!("specialized-{surface}"),
                target: format!("test:{surface}"),
                kind: DeliveryKind::Assistant,
                content: DeliveryContent::Answer(AnswerDraft::from_markdown(source)),
                profile: DeliveryProfile {
                    surface: surface.into(),
                    markup,
                    max_chars: Some(4096),
                    supports_tables: false,
                    supports_code_blocks: true,
                    supports_links: true,
                    supports_actions: false,
                    template: None,
                    posture: DeliveryPosture::default(),
                },
                skill_registry: None,
            };
            let packet = render(&input).expect("specialized output should render");
            assert_eq!(packet.fallback_markdown, source, "surface={surface}");
        }
    }

    #[test]
    fn every_supported_chat_surface_keeps_readable_output_and_fallback() {
        let source =
            "# Plan\n\nA short answer with [a link](https://example.com).\n\n- One\n- Two\n";
        for (surface, markup) in [
            ("telegram", Markup::TelegramHtml),
            ("slack", Markup::SlackMrkdwn),
            ("discord", Markup::DiscordMarkdown),
        ] {
            let mut input = job(markup, Some(256));
            input.content = DeliveryContent::Answer(AnswerDraft::from_markdown(source));
            input.profile.surface = surface.into();
            let packet = render(&input).expect("supported chat surface renders");
            assert_eq!(packet.fallback_markdown, source);
            assert!(!packet.chunks.is_empty(), "{surface} emitted no chunks");
            assert!(
                packet.chunks.iter().all(|chunk| !chunk.trim().is_empty()),
                "{surface} emitted an empty chunk"
            );
            assert!(
                packet
                    .chunks
                    .iter()
                    .all(|chunk| chunk.chars().count() <= 256)
            );
        }
    }

    #[test]
    fn packet_preserves_outcome_metadata_for_channel_consumers() {
        let mut input = job(Markup::Markdown, None);
        let DeliveryContent::Answer(answer) = &mut input.content else {
            panic!("test job must contain an answer");
        };
        answer
            .metadata
            .insert("outcome_completion".into(), "partial".into());
        answer
            .document
            .metadata
            .insert("outcome_evidence_state".into(), "stale".into());
        let packet = render(&input).expect("valid delivery job");
        let DeliveryContent::Answer(answer) = &input.content else {
            panic!("test job must contain an answer");
        };
        assert_eq!(
            answer
                .metadata
                .get("outcome_completion")
                .map(String::as_str),
            Some("partial")
        );
        assert_eq!(
            packet
                .presentation
                .as_ref()
                .and_then(|timeline| timeline.items.first())
                .and_then(|item| match &item.content {
                    OutputContent::Document { document } => document
                        .metadata
                        .get("outcome_evidence_state")
                        .map(String::as_str),
                    _ => None,
                }),
            Some("stale")
        );
        let presentation = packet.presentation.as_ref().expect("presentation");
        assert_eq!(presentation.items[0].status, OutputStatus::Partial);
        assert!(
            presentation
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("incomplete or unverified"))
        );
    }

    #[test]
    fn packet_export_round_trip_preserves_ordered_chunks_and_exact_fallback() {
        // This transport-neutral fixture is the contract consumed by webhook,
        // share/export, and chat adapters: presentation is optional, while
        // chunks and the source fallback remain lossless and ordered.
        let input = job(Markup::Markdown, Some(18));
        let packet = render(&input).expect("valid delivery job");
        let encoded = serde_json::to_vec(&packet).expect("packet is exportable");
        let decoded: DeliveryPacket =
            serde_json::from_slice(&encoded).expect("export must be importable");
        assert_eq!(decoded.fallback_markdown, packet.fallback_markdown);
        assert_eq!(decoded.chunks, packet.chunks);
        assert_eq!(
            decoded.chunks.concat(),
            packet.chunks.concat(),
            "chunk order must survive sharing/export"
        );
        assert_eq!(decoded.presentation, packet.presentation);
    }

    #[test]
    fn typed_results_keep_outcomes_independent() {
        let mut input = job(Markup::Markdown, None);
        let DeliveryContent::Answer(answer) = &mut input.content else {
            panic!("test job must contain an answer");
        };
        answer.results = vec![
            AnswerResult {
                id: "answer".into(),
                source_markdown: "The answer is verified.".into(),
                outcome: Some(ResultOutcome {
                    result_id: "answer".into(),
                    status: OutputStatus::Succeeded,
                    completion: Some("complete".into()),
                    evidence_state: Some("fresh".into()),
                    requirement_ids: vec!["r1".into()],
                    evidence_receipt_ids: vec!["receipt-1".into()],
                    evidence: Vec::new(),
                    human_review: None,
                }),
            },
            AnswerResult {
                id: "artifact".into(),
                source_markdown: "The artifact still needs review.".into(),
                outcome: Some(ResultOutcome {
                    result_id: "artifact".into(),
                    status: OutputStatus::Partial,
                    completion: Some("partial".into()),
                    evidence_state: Some("stale".into()),
                    requirement_ids: vec!["r2".into()],
                    evidence_receipt_ids: Vec::new(),
                    evidence: Vec::new(),
                    human_review: Some("required".into()),
                }),
            },
        ];
        let packet = render(&input).expect("valid delivery job");
        assert!(packet.fallback_markdown.contains("The answer is verified."));
        assert!(
            packet
                .fallback_markdown
                .contains("artifact still needs review")
        );
        assert!(matches!(
            &packet.payload,
            DeliveryPayload::Text(text) if text.contains("Result 'artifact'")
        ));
        let timeline = packet.presentation.expect("presentation");
        assert_eq!(timeline.items.len(), 2);
        assert_eq!(timeline.items[0].status, OutputStatus::Succeeded);
        assert_eq!(timeline.items[1].status, OutputStatus::Partial);
        assert_eq!(
            timeline.items[1]
                .outcome
                .as_ref()
                .and_then(|o| o.human_review.as_deref()),
            Some("required")
        );
        assert_eq!(timeline.diagnostics.len(), 1);
    }

    #[test]
    fn stale_evidence_downgrades_even_a_complete_claim() {
        let mut input = job(Markup::Markdown, None);
        let DeliveryContent::Answer(answer) = &mut input.content else {
            panic!("test job must contain an answer");
        };
        answer
            .metadata
            .insert("outcome_completion".into(), "complete".into());
        answer
            .metadata
            .insert("outcome_evidence_state".into(), "stale".into());
        let packet = render(&input).expect("valid delivery job");
        let presentation = packet.presentation.as_ref().expect("presentation");
        assert_eq!(presentation.items[0].status, OutputStatus::Partial);
        assert!(
            matches!(&packet.payload, DeliveryPayload::Text(text) if text.contains("Outcome incomplete or unverified"))
        );
    }

    #[test]
    fn stale_evidence_warning_survives_messaging_renderers() {
        for markup in [
            Markup::TelegramHtml,
            Markup::SlackMrkdwn,
            Markup::DiscordMarkdown,
        ] {
            let mut input = job(markup, None);
            let DeliveryContent::Answer(answer) = &mut input.content else {
                panic!("test job must contain an answer");
            };
            answer
                .metadata
                .insert("outcome_completion".into(), "complete".into());
            answer
                .metadata
                .insert("outcome_evidence_state".into(), "stale".into());
            let packet = render(&input).expect("valid delivery job");
            assert!(
                matches!(&packet.payload, DeliveryPayload::Text(text) if text.contains("Outcome incomplete or unverified")),
                "markup {markup:?}: {:?}",
                packet.payload
            );
        }
    }

    #[test]
    fn typed_result_warning_survives_all_text_channels() {
        for markup in [
            Markup::Plain,
            Markup::Markdown,
            Markup::TelegramHtml,
            Markup::SlackMrkdwn,
            Markup::DiscordMarkdown,
        ] {
            let mut input = job(markup, None);
            let DeliveryContent::Answer(answer) = &mut input.content else {
                panic!("test job must contain an answer");
            };
            answer.results.push(AnswerResult {
                id: "needs-review".into(),
                source_markdown: "A result with an evidence gap.".into(),
                outcome: Some(ResultOutcome {
                    result_id: "needs-review".into(),
                    status: OutputStatus::Partial,
                    completion: Some("partial".into()),
                    evidence_state: Some("unknown".into()),
                    requirement_ids: vec!["evidence".into()],
                    evidence_receipt_ids: Vec::new(),
                    evidence: Vec::new(),
                    human_review: Some("required".into()),
                }),
            });
            let packet = render(&input).expect("valid delivery job");
            assert!(matches!(
                &packet.payload,
                DeliveryPayload::Text(text) if text.contains("Result 'needs-review'")
            ));
        }
    }

    #[test]
    fn obsolete_answer_schema_is_refused_without_migration() {
        let mut input = job(Markup::Plain, None);
        let DeliveryContent::Answer(answer) = &mut input.content else {
            panic!("test job must contain an answer");
        };
        answer.schema_version = 1;
        answer.document = PresentationDocument::default();
        assert!(matches!(
            render(&input),
            Err(DeliveryError::UnsupportedSchema(1))
        ));
    }

    #[test]
    fn chunking_is_unicode_safe_and_prefers_line_boundaries() {
        let input = DeliveryJob {
            job_id: "job-2".into(),
            target: "test:two".into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(AnswerDraft::from_markdown("éééé\nzzzz")),
            profile: DeliveryProfile {
                max_chars: Some(4),
                ..DeliveryProfile::plain("test")
            },
            skill_registry: None,
        };
        let packet = render(&input).expect("valid delivery job");
        assert!(packet.chunks.iter().all(|chunk| chunk.chars().count() <= 4));
        assert_eq!(packet.chunks, vec!["éééé", "zzzz"]);
    }

    #[test]
    fn user_template_can_replace_layout_without_touching_source() {
        let mut input = job(Markup::Plain, None);
        input.profile.template = Some(TemplateSpec {
            id: "compact-result".into(),
            revision: 1,
            origin: TemplateOrigin::User,
            activation: TemplateActivation::Active,
            nodes: vec![
                TemplateNode::Literal { text: "[".into() },
                TemplateNode::Slot {
                    slot: TemplateSlot::Title,
                },
                TemplateNode::Literal { text: "]\n".into() },
                TemplateNode::Slot {
                    slot: TemplateSlot::Body,
                },
            ],
        });
        let packet = render(&input).expect("valid template");
        let DeliveryPayload::Text(text) = packet.payload else {
            panic!("plain profile must produce text");
        };
        assert!(text.starts_with("[Result]\n"));
        assert_eq!(
            packet.fallback_markdown,
            "# Result\n\nDone.\n\n```rust\nlet x = 1;\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n"
        );
    }

    #[test]
    fn agent_template_requires_activation_and_user_templates_win() {
        let mut registry = TemplateRegistry::default();
        registry
            .upsert(TemplateSpec {
                id: "result".into(),
                revision: 2,
                origin: TemplateOrigin::AgentProposal,
                activation: TemplateActivation::Proposed,
                nodes: Vec::new(),
            })
            .expect("valid proposal");
        assert!(registry.resolve("result").is_none());
        assert!(registry.activate_proposal("result", 2));
        assert_eq!(registry.resolve("result").map(|t| t.revision), Some(2));

        registry
            .upsert(TemplateSpec {
                id: "result".into(),
                revision: 1,
                origin: TemplateOrigin::User,
                activation: TemplateActivation::Active,
                nodes: Vec::new(),
            })
            .expect("valid user template");
        assert_eq!(
            registry.resolve("result").map(|t| t.origin),
            Some(TemplateOrigin::User)
        );
    }

    #[test]
    fn control_plane_messages_cannot_be_templated_or_flattened() {
        let input = DeliveryJob {
            job_id: "approval-1".into(),
            target: "test:approval".into(),
            kind: DeliveryKind::System,
            content: DeliveryContent::Text {
                markdown: "system instruction".into(),
            },
            profile: DeliveryProfile::plain("test"),
            skill_registry: None,
        };
        let error = render(&input).expect_err("system content must stay on event stream");
        assert!(error.to_string().contains("control-plane"));
    }

    #[test]
    fn approval_actions_survive_text_rendering() {
        let action = DeliveryAction {
            id: "approve".into(),
            label: "Approve".into(),
            verb: "approve".into(),
            data: BTreeMap::new(),
        };
        let input = DeliveryJob {
            job_id: "approval-2".into(),
            target: "test:approval".into(),
            kind: DeliveryKind::Approval,
            content: DeliveryContent::Approval(ApprovalPayload {
                request_id: "request-1".into(),
                title: "Run command?".into(),
                detail: "echo hello".into(),
                expires_at: None,
                actions: vec![action],
            }),
            profile: DeliveryProfile::plain("test"),
            skill_registry: None,
        };
        let packet = render(&input).expect("approval should render");
        assert_eq!(packet.actions.len(), 1);
        assert!(
            packet
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("actions preserved"))
        );
    }

    #[test]
    fn worker_returns_values_for_bad_input() {
        let response = worker::process_line("not-json");
        assert!(response.contains("invalid delivery worker request"));
    }

    fn fallback_text(packet: &DeliveryPacket) -> String {
        match &packet.payload {
            DeliveryPayload::Text(text) => text.clone(),
            DeliveryPayload::Structured(content) => match content {
                DeliveryContent::Answer(answer) => answer.source_markdown.clone(),
                DeliveryContent::Text { markdown } => markdown.clone(),
                DeliveryContent::Progress(progress) => {
                    format!("{}: {}", progress.label, progress.state)
                }
                DeliveryContent::ToolResult(payload) => payload.output.clone(),
                _ => packet.fallback_markdown.clone(),
            },
        }
    }

    #[test]
    fn structured_fence_projected_on_telegram_html() {
        let source = "# Result\n\n```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"tokens\",\"value\":128}}\n```\n\nSummary text here.\n";
        let input = DeliveryJob {
            job_id: "struct-1".into(),
            target: "test:one".into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(AnswerDraft::from_markdown(source)),
            profile: DeliveryProfile {
                surface: "telegram".into(),
                markup: Markup::TelegramHtml,
                max_chars: Some(4000),
                supports_tables: false,
                supports_code_blocks: true,
                supports_links: true,
                supports_actions: false,
                template: None,
                posture: DeliveryPosture::default(),
            },
            skill_registry: None,
        };
        let packet = render(&input).expect("valid delivery job");
        let text = fallback_text(&packet);
        assert!(
            text.contains("tokens"),
            "projected text should contain the metric label"
        );
        assert!(
            text.contains("128"),
            "projected text should contain the metric value"
        );
        assert!(
            text.contains("Summary text here"),
            "non-fence content preserved"
        );
        assert!(
            !text.contains("```vak"),
            "raw vak fence must be projected, not shown verbatim"
        );
    }

    #[test]
    fn structured_fence_projected_on_slack_mrkdwn() {
        let source = "Done.\n\n```vak\n{\"semantic_type\":\"link.preview\",\"payload\":{\"url\":\"https://example.com\",\"title\":\"Example\"}}\n```\n";
        let input = DeliveryJob {
            job_id: "struct-2".into(),
            target: "test:one".into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(AnswerDraft::from_markdown(source)),
            profile: DeliveryProfile {
                surface: "slack".into(),
                markup: Markup::SlackMrkdwn,
                max_chars: Some(3900),
                supports_tables: false,
                supports_code_blocks: true,
                supports_links: true,
                supports_actions: false,
                template: None,
                posture: DeliveryPosture::default(),
            },
            skill_registry: None,
        };
        let packet = render(&input).expect("valid delivery job");
        let text = fallback_text(&packet);
        assert!(text.contains("Example"), "link preview title should appear");
        assert!(
            text.contains("https://example.com"),
            "link URL should appear"
        );
        assert!(!text.contains("```vak"), "raw fence must be projected");
    }

    #[test]
    fn structured_fence_projected_on_discord_markdown() {
        let source = "Done.\n\n```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"latency\",\"value\":42}}\n```\n";
        let input = DeliveryJob {
            job_id: "struct-3".into(),
            target: "test:one".into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(AnswerDraft::from_markdown(source)),
            profile: DeliveryProfile {
                surface: "discord".into(),
                markup: Markup::DiscordMarkdown,
                max_chars: Some(1900),
                supports_tables: false,
                supports_code_blocks: true,
                supports_links: true,
                supports_actions: false,
                template: None,
                posture: DeliveryPosture::default(),
            },
            skill_registry: None,
        };
        let packet = render(&input).expect("valid delivery job");
        let text = fallback_text(&packet);
        assert!(text.contains("latency"), "metric label should appear");
        assert!(text.contains("42"), "metric value should appear");
        assert!(!text.contains("```vak"), "raw fence must be projected");
    }

    #[test]
    fn structured_fence_passed_through_as_json_for_native_surface() {
        let source = "Done.\n\n```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"tokens\",\"value\":128}}\n```\n";
        let input = DeliveryJob {
            job_id: "struct-4".into(),
            target: "test:one".into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(AnswerDraft::from_markdown(source)),
            profile: DeliveryProfile {
                surface: "desktop".into(),
                markup: Markup::Json,
                max_chars: None,
                supports_tables: true,
                supports_code_blocks: true,
                supports_links: true,
                supports_actions: true,
                template: None,
                posture: DeliveryPosture::default(),
            },
            skill_registry: None,
        };
        let packet = render(&input).expect("valid delivery job");
        // For Json markup, the structured fence should NOT be projected
        // — it stays as a structured content block for the desktop renderer.
        let text = fallback_text(&packet);
        assert!(
            text.contains("```vak"),
            "Json surface must preserve raw fence for AST rendering"
        );
    }

    #[test]
    fn tool_result_with_vak_fence_is_projected() {
        let tool_output = "Command completed.\n\n```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"exit_code\",\"value\":0}}\n```\n";
        let content = DeliveryContent::ToolResult(ToolResultPayload {
            tool: "bash".into(),
            output: tool_output.into(),
            is_error: false,
        });
        let input = DeliveryJob {
            job_id: "tool-1".into(),
            target: "test:one".into(),
            kind: DeliveryKind::ToolResult,
            content,
            profile: DeliveryProfile {
                surface: "test".into(),
                markup: Markup::TelegramHtml,
                max_chars: Some(4000),
                supports_tables: false,
                supports_code_blocks: true,
                supports_links: true,
                supports_actions: false,
                template: None,
                posture: DeliveryPosture::default(),
            },
            skill_registry: None,
        };
        let packet = render(&input).expect("valid tool result job");
        let text = fallback_text(&packet);
        assert!(
            text.contains("exit_code"),
            "tool result metric label should appear"
        );
        assert!(text.contains("0"), "tool result metric value should appear");
        assert!(text.contains("bash"), "tool name should appear in header");
    }

    #[test]
    fn malformed_vak_fence_is_preserved_as_code_block() {
        let source = "Done.\n\n```vak\n{this is not json}\n```\n";
        let input = DeliveryJob {
            job_id: "struct-5".into(),
            target: "test:one".into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(AnswerDraft::from_markdown(source)),
            profile: DeliveryProfile {
                surface: "test".into(),
                markup: Markup::TelegramHtml,
                max_chars: Some(4000),
                supports_tables: false,
                supports_code_blocks: true,
                supports_links: true,
                supports_actions: false,
                template: None,
                posture: DeliveryPosture::default(),
            },
            skill_registry: None,
        };
        let result = render(&input);
        // Must not panic on malformed JSON inside the fence
        assert!(
            result.is_ok(),
            "malformed fence should not crash the worker"
        );
    }

    #[test]
    fn plugin_contributed_skill_is_recognized() {
        use std::collections::BTreeMap;
        let mut registry = built_in_skill_registry();
        let mut renderers = BTreeMap::new();
        renderers.insert(
            "desktop".to_string(),
            RendererBinding {
                renderer: "metric".into(),
                interactive: false,
                requires: vec![],
            },
        );
        let plugin_manifest = PresentationSkillManifest {
            id: "custom_metric".into(),
            version: "1.0.0".into(),
            api: PRESENTATION_SKILL_API.into(),
            provides: vec!["custom.metric".into()],
            renderers,
            schema: None,
            outcome_requirements: Vec::new(),
        };
        let files = vec![(
            "custom_skill.json".to_string(),
            serde_json::to_vec(&plugin_manifest).expect("plugin manifest must serialize"),
        )];
        registry.merge_plugin_files(&files);
        assert!(registry.get("custom_metric").is_some());
        // The plugin skill should be used when parsing its semantic type:
        // skill_id is resolved from the registry, not hardcoded to "core".
        let fence =
            r#"{"semantic_type":"custom.metric","payload":{"label":"throughput","value":42}}"#;
        let output = parse_fragment_with(fence, &registry)
            .expect("plugin skill should parse with correct skill_id");
        assert_eq!(output.skill_id, "custom_metric");
        assert_eq!(output.skill_version, "1.0.0");
        assert_eq!(output.semantic_type, "custom.metric");
    }
}

// ---- engagement posture (docs/design/47-commitment-kernel.md) --------------

/// When a delivery should reach its reader, and how hard it may push.
///
/// The one behaviour worth having: an unattended overnight run must not send
/// forty notifications nobody reads. Nothing about the *content* changes — the
/// semantic contract, the renderer, and the outbox are untouched — this only
/// decides whether a packet goes out now, waits for the unit of work to
/// finish, or rolls into the next digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Cadence {
    /// Stream as it happens. Somebody is watching.
    Live,
    /// One delivery when the unit of work finishes.
    OnCompletion,
    /// Roll up into the next digest.
    Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Urgency {
    /// Break through whatever the cadence says. Reserved for things a person
    /// would want woken for.
    Interrupt,
    Notify,
    Quiet,
}

/// How a turn's engagement wants its output delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryPosture {
    pub cadence: Cadence,
    pub urgency: Urgency,
}

impl Default for DeliveryPosture {
    fn default() -> Self {
        // The pre-kernel behaviour: everything goes out immediately.
        DeliveryPosture {
            cadence: Cadence::Live,
            urgency: Urgency::Notify,
        }
    }
}

impl DeliveryPosture {
    /// Convert the intent kernel's serialized cadence/urgency labels at the
    /// delivery boundary without introducing a dependency cycle.
    pub fn from_intent_labels(cadence: &str, urgency: &str) -> Self {
        let cadence = match cadence {
            "on-completion" | "on_completion" => Cadence::OnCompletion,
            "digest" => Cadence::Digest,
            _ => Cadence::Live,
        };
        let urgency = match urgency {
            "interrupt" => Urgency::Interrupt,
            "quiet" => Urgency::Quiet,
            _ => Urgency::Notify,
        };
        Self { cadence, urgency }
    }
}

/// What to do with one packet under a posture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Send now.
    Send,
    /// Hold until the unit of work completes.
    HoldUntilComplete,
    /// Hold for the next digest.
    HoldForDigest,
}

impl DeliveryPosture {
    /// Decide one packet's fate.
    ///
    /// Two rules override the cadence, and both are about not losing something
    /// that matters:
    ///
    /// * an `Interrupt` urgency always sends — an irreversible step's
    ///   confirmation must not sit in a digest until morning;
    /// * anything that needs a person *now* (an approval gate) always sends,
    ///   because a held gate is a stopped run, and batching it would turn a
    ///   question into a hang.
    pub fn disposition(&self, kind: DeliveryKind) -> Disposition {
        if self.urgency == Urgency::Interrupt || needs_a_person_now(kind) {
            return Disposition::Send;
        }
        match self.cadence {
            Cadence::Live => Disposition::Send,
            Cadence::OnCompletion => match kind {
                // Progress and tool chatter are the noise; the answer itself
                // is the thing that was waited for.
                DeliveryKind::Progress | DeliveryKind::ToolResult | DeliveryKind::ToolCall => {
                    Disposition::HoldUntilComplete
                }
                _ => Disposition::Send,
            },
            Cadence::Digest => match kind {
                DeliveryKind::Alert => Disposition::Send,
                _ => Disposition::HoldForDigest,
            },
        }
    }
}

/// Deliveries that stop a run until somebody answers. Never batched.
fn needs_a_person_now(kind: DeliveryKind) -> bool {
    matches!(kind, DeliveryKind::Approval)
}

#[cfg(test)]
mod posture_tests {
    use super::*;

    fn posture(cadence: Cadence, urgency: Urgency) -> DeliveryPosture {
        DeliveryPosture { cadence, urgency }
    }

    #[test]
    fn intent_labels_map_to_delivery_posture() {
        assert_eq!(
            DeliveryPosture::from_intent_labels("on-completion", "quiet"),
            posture(Cadence::OnCompletion, Urgency::Quiet)
        );
        assert_eq!(
            DeliveryPosture::from_intent_labels("unknown", "unknown"),
            DeliveryPosture::default()
        );
    }

    /// The behaviour this exists for: overnight work sends a roll-up, not a
    /// stream of progress nobody is awake to read.
    #[test]
    fn an_unattended_run_holds_its_chatter_for_the_digest() {
        let quiet = posture(Cadence::Digest, Urgency::Quiet);
        assert_eq!(
            quiet.disposition(DeliveryKind::Progress),
            Disposition::HoldForDigest
        );
        assert_eq!(
            quiet.disposition(DeliveryKind::Assistant),
            Disposition::HoldForDigest
        );
    }

    /// A held approval gate is a stopped run. Batching it would turn a
    /// question into a hang, so it always goes out.
    #[test]
    fn an_approval_always_goes_out_whatever_the_cadence() {
        for cadence in [Cadence::Live, Cadence::OnCompletion, Cadence::Digest] {
            for urgency in [Urgency::Interrupt, Urgency::Notify, Urgency::Quiet] {
                assert_eq!(
                    posture(cadence, urgency).disposition(DeliveryKind::Approval),
                    Disposition::Send,
                    "{cadence:?}/{urgency:?} batched an approval"
                );
            }
        }
    }

    /// An irreversible step's confirmation must not wait until morning.
    #[test]
    fn interrupt_urgency_overrides_every_cadence() {
        let urgent = posture(Cadence::Digest, Urgency::Interrupt);
        assert_eq!(
            urgent.disposition(DeliveryKind::Assistant),
            Disposition::Send
        );
        assert_eq!(
            urgent.disposition(DeliveryKind::Progress),
            Disposition::Send
        );
    }

    /// Alerts are already the exception path; a digest must not swallow one.
    #[test]
    fn an_alert_is_never_held_for_a_digest() {
        assert_eq!(
            posture(Cadence::Digest, Urgency::Quiet).disposition(DeliveryKind::Alert),
            Disposition::Send
        );
    }

    /// On-completion holds the chatter and sends the answer.
    #[test]
    fn on_completion_holds_progress_but_not_the_result() {
        let posture = posture(Cadence::OnCompletion, Urgency::Notify);
        assert_eq!(
            posture.disposition(DeliveryKind::Progress),
            Disposition::HoldUntilComplete
        );
        assert_eq!(
            posture.disposition(DeliveryKind::Assistant),
            Disposition::Send
        );
    }

    /// The default reproduces the behaviour before any of this existed.
    #[test]
    fn the_default_posture_sends_everything() {
        let default = DeliveryPosture::default();
        for kind in [
            DeliveryKind::Assistant,
            DeliveryKind::Progress,
            DeliveryKind::Alert,
            DeliveryKind::Approval,
            DeliveryKind::ToolResult,
        ] {
            assert_eq!(default.disposition(kind), Disposition::Send);
        }
    }
}
