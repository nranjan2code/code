//! Loss-accounting output engineering for channel delivery.
//!
//! The crate deliberately has no transport or server dependencies. It turns
//! a complete answer draft into a bounded, channel-specific packet in a
//! separate process. The source Markdown remains part of every packet so a
//! renderer can never become the system of record.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod client;
pub mod outbox;
pub mod presentation;
pub mod skills;
pub mod telegram;
pub mod templates;

pub use presentation::{
    ArtifactRef, CalloutTone, Citation, DocumentBlock, DocumentCoverage,
    DocumentCoverageDisposition, InlineNode, OutputContent, OutputItem, OutputKind,
    OutputProvenance, OutputRole, OutputStatus, OutputStreamEvent, OutputTimeline,
    PresentationDocument, SurfaceCapabilities, TableAlignment, compile_markdown, inline_text,
    safe_link,
};
pub use skills::{
    ChartOutput, ChartPoint, ChartSeries, DecisionDisposition, LinkPreview, MediaOutput, Metric,
    PRESENTATION_SKILL_API, PlanDiagnostic, PresentationDecision, PresentationPlan,
    PresentationPlanner, PresentationRecipe, PresentationSkillManifest, RecipeCatalog,
    RendererBinding, SkillError, SkillRegistry, StructuredOutput, built_in_recipes,
    built_in_skill_registry, link_previews_from_text, signals_from_text,
};

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
        }
    }

    fn presentation_document(&self) -> PresentationDocument {
        if self.document.source_markdown == self.source_markdown && !self.document.blocks.is_empty()
        {
            self.document.clone()
        } else {
            compile_markdown(self.source_markdown.clone())
        }
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

    fn render(&self, answer: &AnswerDraft) -> Result<String, DeliveryError> {
        self.validate()?;
        render_nodes(&self.nodes, answer, 0)
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
            TemplateNode::Slot { slot } => out.push_str(&slot_value(slot, answer)),
            TemplateNode::IfPresent {
                slot,
                then_nodes,
                else_nodes,
            } => {
                let selected = if slot_value(slot, answer).is_empty() {
                    else_nodes
                } else {
                    then_nodes
                };
                out.push_str(&render_nodes(selected, answer, depth + 1)?);
            }
        }
    }
    Ok(out)
}

fn slot_value(slot: &TemplateSlot, answer: &AnswerDraft) -> String {
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
        TemplateSlot::Body => render_plain(answer),
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
        && answer.schema_version != 1
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
    let chunks = if matches!(job.profile.markup, Markup::TelegramHtml) {
        telegram::split_html_chunks(&rendered, job.profile.max_chars)
    } else {
        chunk_text(&rendered, job.profile.max_chars)
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
    let document = answer.presentation_document();
    Some(OutputTimeline {
        schema_version: presentation::PRESENTATION_SCHEMA_VERSION,
        session_id: String::new(),
        cursor: None,
        items: vec![OutputItem {
            id: job.job_id.clone(),
            timestamp: String::new(),
            turn_id: job.job_id.clone(),
            role: OutputRole::Assistant,
            kind: match job.kind {
                DeliveryKind::Assistant | DeliveryKind::TaskSummary | DeliveryKind::Alert => {
                    OutputKind::Outcome
                }
                _ => OutputKind::Message,
            },
            status: OutputStatus::Succeeded,
            content: OutputContent::Document { document },
            provenance: None,
            actions: Vec::new(),
            fallback_text: answer.source_markdown.clone(),
        }],
        diagnostics: Vec::new(),
    })
}

fn render_content(
    job: &DeliveryJob,
) -> Result<(String, String, Vec<DeliveryAction>, Vec<Coverage>), DeliveryError> {
    match &job.content {
        DeliveryContent::Answer(answer) => {
            let rendered = match job.profile.template.as_ref() {
                Some(template) => render_text(&template.render(answer)?, job.profile.markup),
                None => render_answer(answer, job.profile.markup),
            };
            let coverage = answer
                .blocks
                .iter()
                .map(|block| Coverage {
                    block_id: block.id().to_string(),
                    disposition: CoverageDisposition::Rendered,
                })
                .collect();
            Ok((
                rendered,
                answer.source_markdown.clone(),
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
            format!("{}: {}", progress.label, progress.state),
            format!("{}: {}", progress.label, progress.state),
            Vec::new(),
            Vec::new(),
        )),
        DeliveryContent::ToolResult(result) => Ok((
            format_tool_result(result),
            result.output.clone(),
            Vec::new(),
            Vec::new(),
        )),
        DeliveryContent::Text { markdown } => Ok((
            render_text(markdown, job.profile.markup),
            markdown.clone(),
            Vec::new(),
            Vec::new(),
        )),
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

fn render_answer(answer: &AnswerDraft, markup: Markup) -> String {
    match markup {
        Markup::Plain => render_plain(answer),
        Markup::Markdown => answer.source_markdown.clone(),
        Markup::TelegramHtml => telegram::markdown_to_html(&answer.source_markdown),
        Markup::Json => String::new(),
    }
}

fn render_text(markdown: &str, markup: Markup) -> String {
    match markup {
        Markup::TelegramHtml => telegram::markdown_to_html(markdown),
        Markup::Plain | Markup::Markdown | Markup::Json => markdown.to_string(),
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

fn format_tool_result(result: &ToolResultPayload) -> String {
    let prefix = if result.is_error {
        "Tool error"
    } else {
        "Tool result"
    };
    format!("{prefix} ({})\n\n{}", result.tool, result.output)
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

fn render_plain(answer: &AnswerDraft) -> String {
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
            },
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
    fn schema_one_answer_is_compiled_without_source_loss() {
        let mut input = job(Markup::Plain, None);
        let DeliveryContent::Answer(answer) = &mut input.content else {
            panic!("test job must contain an answer");
        };
        answer.schema_version = 1;
        answer.document = PresentationDocument::default();
        let source = answer.source_markdown.clone();

        let packet = render(&input).expect("schema one remains readable");
        let timeline = packet
            .presentation
            .expect("legacy input gets a v2 projection");
        let OutputContent::Document { document } = &timeline.items[0].content else {
            panic!("legacy answer must compile to a document");
        };
        assert_eq!(document.source_markdown, source);
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
}
