use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PRESENTATION_SCHEMA_VERSION: u16 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputRole {
    System,
    User,
    Assistant,
    Tool,
    Subagent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    Message,
    Information,
    Approval,
    Progress,
    Retry,
    Error,
    Outcome,
    Artifact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Denied,
    Cancelled,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputProvenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputItem {
    pub id: String,
    pub timestamp: String,
    pub turn_id: String,
    pub role: OutputRole,
    pub kind: OutputKind,
    pub status: OutputStatus,
    /// The machine-readable result contract for this item. Presentation is
    /// result-scoped: one answer may contain several independently evaluated
    /// results, rather than one status being painted over the whole answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ResultOutcome>,
    pub content: OutputContent,
    #[serde(default)]
    pub provenance: Option<OutputProvenance>,
    #[serde(default)]
    pub actions: Vec<crate::DeliveryAction>,
    pub fallback_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultOutcome {
    pub result_id: String,
    pub status: OutputStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_state: Option<String>,
    #[serde(default)]
    pub requirement_ids: Vec<String>,
    #[serde(default)]
    pub evidence_receipt_ids: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<vak_intent::EvidenceReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_review: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputContent {
    Document {
        document: PresentationDocument,
    },
    Information {
        label: String,
        detail: Option<String>,
    },
    Approval {
        request_id: String,
        tool: String,
        args_json: String,
        reason: String,
        expires_at: Option<String>,
    },
    Progress {
        label: String,
        detail: Option<String>,
        percent: Option<u8>,
    },
    Retry {
        attempt: u32,
        delay_ms: u64,
        reason: String,
    },
    Error {
        message: String,
        source: Option<String>,
        retryable: bool,
    },
    Outcome {
        summary: String,
        document: Option<PresentationDocument>,
    },
    Artifact {
        artifact: ArtifactRef,
    },
    /// Validated data supplied by an installed presentation skill. The
    /// renderer registry, never the model, decides how this is displayed.
    Structured {
        output: crate::skills::StructuredOutput,
    },
    /// A host-compiled adaptive tree. The original fallback remains attached
    /// so constrained clients and voice narration can lower it safely.
    Adaptive {
        tree: vak_presentation::RenderTree,
        fallback_text: String,
    },
}

impl OutputContent {
    /// Build the canonical adaptive content envelope for a specialized
    /// renderer.  Degraded compilations deliberately remain on the caller's
    /// existing document path so the original renderer stays authoritative.
    pub fn from_compiled_adaptive(
        compiled: vak_presentation::CompiledPresentation,
        fallback_text: impl Into<String>,
    ) -> Option<Self> {
        match compiled {
            vak_presentation::CompiledPresentation::Rich(tree) => Some(Self::Adaptive {
                fallback_text: fallback_text.into(),
                tree,
            }),
            vak_presentation::CompiledPresentation::Fallback { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputTimeline {
    pub schema_version: u16,
    pub session_id: String,
    #[serde(default)]
    pub cursor: Option<String>,
    pub items: Vec<OutputItem>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<vak_intent::GoalState>,
}

impl OutputTimeline {
    pub fn empty(session_id: impl Into<String>) -> Self {
        Self {
            schema_version: PRESENTATION_SCHEMA_VERSION,
            session_id: session_id.into(),
            cursor: None,
            items: Vec::new(),
            diagnostics: Vec::new(),
            goal: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputStreamEvent {
    Snapshot {
        timeline: OutputTimeline,
    },
    ItemStarted {
        item: OutputItem,
    },
    TextDelta {
        item_id: String,
        delta: String,
        text: String,
    },
    ItemReplaced {
        item: OutputItem,
    },
    ItemCompleted {
        item_id: String,
        status: OutputStatus,
    },
}

/// A reconnectable presentation frame. Consumers may apply the delta or
/// replace their state with the complete snapshot; both describe the same point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputStreamFrame {
    pub sequence: Option<u64>,
    pub delta: Option<OutputStreamEvent>,
    pub snapshot: OutputTimeline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceCapabilities {
    pub structured_blocks: bool,
    pub tables: bool,
    pub code: bool,
    pub links: bool,
    pub media: bool,
    pub file_references: bool,
    pub actions: bool,
    pub color: bool,
    pub interactive: bool,
    pub accessible_plain: bool,
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationDocument {
    pub schema_version: u16,
    pub source_markdown: String,
    pub blocks: Vec<DocumentBlock>,
    #[serde(default)]
    pub coverage: Vec<DocumentCoverage>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

impl Default for PresentationDocument {
    fn default() -> Self {
        Self {
            schema_version: PRESENTATION_SCHEMA_VERSION,
            source_markdown: String::new(),
            blocks: Vec::new(),
            coverage: Vec::new(),
            metadata: BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentCoverage {
    pub block_id: String,
    pub disposition: DocumentCoverageDisposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentCoverageDisposition {
    Native,
    Fallback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DocumentBlock {
    Heading {
        id: String,
        level: u8,
        content: Vec<InlineNode>,
    },
    Paragraph {
        id: String,
        content: Vec<InlineNode>,
    },
    List {
        id: String,
        ordered: bool,
        start: Option<u64>,
        items: Vec<Vec<DocumentBlock>>,
    },
    Table {
        id: String,
        alignments: Vec<TableAlignment>,
        header: Vec<Vec<InlineNode>>,
        rows: Vec<Vec<Vec<InlineNode>>>,
    },
    Quote {
        id: String,
        blocks: Vec<DocumentBlock>,
    },
    Code {
        id: String,
        language: Option<String>,
        filename: Option<String>,
        content: String,
    },
    Structured {
        id: String,
        output: crate::skills::StructuredOutput,
        fallback_markdown: String,
    },
    Diagram {
        id: String,
        source: String,
        fallback_markdown: String,
    },
    Callout {
        id: String,
        tone: CalloutTone,
        title: Option<String>,
        blocks: Vec<DocumentBlock>,
    },
    Diff {
        id: String,
        content: String,
    },
    Citations {
        id: String,
        items: Vec<Citation>,
    },
    Media {
        id: String,
        source: String,
        alt: String,
        media_type: Option<String>,
    },
    ArtifactRef {
        id: String,
        artifact: ArtifactRef,
    },
    Rule {
        id: String,
    },
    RawMarkdown {
        id: String,
        markdown: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InlineNode {
    Text {
        text: String,
    },
    Strong {
        content: Vec<InlineNode>,
    },
    Emphasis {
        content: Vec<InlineNode>,
    },
    Strikethrough {
        content: Vec<InlineNode>,
    },
    Code {
        code: String,
    },
    Link {
        label: Vec<InlineNode>,
        url: String,
        title: Option<String>,
        safe: bool,
    },
    Image {
        alt: String,
        url: String,
        title: Option<String>,
        safe: bool,
    },
    SoftBreak,
    HardBreak,
    RawHtml {
        html: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableAlignment {
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalloutTone {
    Information,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Citation {
    pub label: String,
    pub url: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub name: String,
    pub path: Option<String>,
    pub media_type: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug)]
enum Node {
    Root(Vec<Node>),
    Paragraph(Vec<Node>),
    Heading(u8, Vec<Node>),
    BlockQuote(Vec<Node>),
    CodeBlock(Option<String>, String),
    List(Option<u64>, Vec<Node>),
    Item(Vec<Node>),
    Table(Vec<TableAlignment>, Vec<Node>),
    TableHead(Vec<Node>),
    TableRow(Vec<Node>),
    TableCell(Vec<Node>),
    Strong(Vec<Node>),
    Emphasis(Vec<Node>),
    Strikethrough(Vec<Node>),
    Link(String, Option<String>, Vec<Node>),
    Image(String, Option<String>, Vec<Node>),
    Text(String),
    InlineCode(String),
    SoftBreak,
    HardBreak,
    Rule,
    RawHtml(String),
    Unsupported(String, Vec<Node>),
}

impl Node {
    fn children_mut(&mut self) -> Option<&mut Vec<Node>> {
        match self {
            Self::Root(v)
            | Self::Paragraph(v)
            | Self::Heading(_, v)
            | Self::BlockQuote(v)
            | Self::List(_, v)
            | Self::Item(v)
            | Self::Table(_, v)
            | Self::TableHead(v)
            | Self::TableRow(v)
            | Self::TableCell(v)
            | Self::Strong(v)
            | Self::Emphasis(v)
            | Self::Strikethrough(v)
            | Self::Link(_, _, v)
            | Self::Image(_, _, v)
            | Self::Unsupported(_, v) => Some(v),
            _ => None,
        }
    }
}

pub fn compile_markdown(source: impl Into<String>) -> PresentationDocument {
    let raw_source = source.into();
    let source_markdown: String = raw_source
        .chars()
        .filter(|c| !('\u{E0000}'..='\u{E007F}').contains(c))
        .collect();
    let mut stack = vec![Node::Root(Vec::new())];
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    for event in Parser::new_ext(&source_markdown, options) {
        match event {
            Event::Start(tag) => stack.push(start_node(tag)),
            Event::End(_) => close_node(&mut stack),
            Event::Text(text) => append_node(&mut stack, Node::Text(text.into_string())),
            Event::Code(code) => append_node(&mut stack, Node::InlineCode(code.into_string())),
            Event::Html(html) | Event::InlineHtml(html) => {
                append_node(&mut stack, Node::RawHtml(html.into_string()));
            }
            Event::SoftBreak => append_node(&mut stack, Node::SoftBreak),
            Event::HardBreak => append_node(&mut stack, Node::HardBreak),
            Event::Rule => append_node(&mut stack, Node::Rule),
            Event::TaskListMarker(checked) => append_node(
                &mut stack,
                Node::Text(if checked { "☑ " } else { "☐ " }.into()),
            ),
            Event::FootnoteReference(label) => {
                append_node(&mut stack, Node::Text(format!("[^{label}]")));
            }
            Event::InlineMath(value) | Event::DisplayMath(value) => {
                append_node(&mut stack, Node::InlineCode(value.into_string()));
            }
        }
    }
    while stack.len() > 1 {
        close_node(&mut stack);
    }
    let nodes = match stack.pop() {
        Some(Node::Root(nodes)) => nodes,
        _ => Vec::new(),
    };
    let mut ids = Ids::default();
    let mut diagnostics = Vec::new();
    let blocks = nodes_to_blocks(nodes, &mut ids, &mut diagnostics);
    let mut coverage = Vec::new();
    collect_coverage(&blocks, &mut coverage);
    PresentationDocument {
        schema_version: PRESENTATION_SCHEMA_VERSION,
        source_markdown,
        blocks,
        coverage,
        metadata: BTreeMap::new(),
        diagnostics,
    }
}

fn collect_coverage(blocks: &[DocumentBlock], coverage: &mut Vec<DocumentCoverage>) {
    for block in blocks {
        let (block_id, disposition, diagnostic, nested) = match block {
            DocumentBlock::Heading { id, .. }
            | DocumentBlock::Paragraph { id, .. }
            | DocumentBlock::Table { id, .. }
            | DocumentBlock::Code { id, .. }
            | DocumentBlock::Structured { id, .. }
            | DocumentBlock::Diagram { id, .. }
            | DocumentBlock::Diff { id, .. }
            | DocumentBlock::Citations { id, .. }
            | DocumentBlock::Media { id, .. }
            | DocumentBlock::ArtifactRef { id, .. }
            | DocumentBlock::Rule { id } => (id, DocumentCoverageDisposition::Native, None, None),
            DocumentBlock::List { id, items, .. } => {
                coverage.push(DocumentCoverage {
                    block_id: id.clone(),
                    disposition: DocumentCoverageDisposition::Native,
                    diagnostic: None,
                });
                for item in items {
                    collect_coverage(item, coverage);
                }
                continue;
            }
            DocumentBlock::Quote { id, blocks } | DocumentBlock::Callout { id, blocks, .. } => (
                id,
                DocumentCoverageDisposition::Native,
                None,
                Some(blocks.as_slice()),
            ),
            DocumentBlock::RawMarkdown { id, reason, .. } => (
                id,
                DocumentCoverageDisposition::Fallback,
                Some(reason.clone()),
                None,
            ),
        };
        coverage.push(DocumentCoverage {
            block_id: block_id.clone(),
            disposition,
            diagnostic,
        });
        if let Some(nested) = nested {
            collect_coverage(nested, coverage);
        }
    }
}

fn start_node(tag: Tag<'_>) -> Node {
    match tag {
        Tag::Paragraph => Node::Paragraph(Vec::new()),
        Tag::Heading { level, .. } => Node::Heading(heading_level(level), Vec::new()),
        Tag::BlockQuote(_) => Node::BlockQuote(Vec::new()),
        Tag::CodeBlock(kind) => Node::CodeBlock(
            match kind {
                CodeBlockKind::Indented => None,
                CodeBlockKind::Fenced(info) => info
                    .split_whitespace()
                    .next()
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
            },
            String::new(),
        ),
        Tag::List(start) => Node::List(start, Vec::new()),
        Tag::Item => Node::Item(Vec::new()),
        Tag::Table(alignments) => Node::Table(
            alignments
                .into_iter()
                .map(|alignment| match alignment {
                    Alignment::None => TableAlignment::None,
                    Alignment::Left => TableAlignment::Left,
                    Alignment::Center => TableAlignment::Center,
                    Alignment::Right => TableAlignment::Right,
                })
                .collect(),
            Vec::new(),
        ),
        Tag::TableHead => Node::TableHead(Vec::new()),
        Tag::TableRow => Node::TableRow(Vec::new()),
        Tag::TableCell => Node::TableCell(Vec::new()),
        Tag::Emphasis => Node::Emphasis(Vec::new()),
        Tag::Strong => Node::Strong(Vec::new()),
        Tag::Strikethrough => Node::Strikethrough(Vec::new()),
        Tag::Link {
            dest_url, title, ..
        } => Node::Link(
            dest_url.into_string(),
            nonempty(title.into_string()),
            Vec::new(),
        ),
        Tag::Image {
            dest_url, title, ..
        } => Node::Image(
            dest_url.into_string(),
            nonempty(title.into_string()),
            Vec::new(),
        ),
        other => Node::Unsupported(format!("{other:?}"), Vec::new()),
    }
}

fn append_node(stack: &mut [Node], node: Node) {
    if let Some(children) = stack.last_mut().and_then(Node::children_mut) {
        children.push(node);
    } else if let Some(Node::CodeBlock(_, content)) = stack.last_mut()
        && let Node::Text(text) = node
    {
        content.push_str(&text);
    }
}

fn close_node(stack: &mut Vec<Node>) {
    if stack.len() < 2 {
        return;
    }
    let Some(node) = stack.pop() else {
        return;
    };
    if let Some(Node::CodeBlock(_, parent)) = stack.last_mut()
        && let Node::Text(text) = node
    {
        parent.push_str(&text);
        return;
    }
    if let Some(children) = stack.last_mut().and_then(Node::children_mut) {
        children.push(node);
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn nonempty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

#[derive(Default)]
struct Ids(usize);

impl Ids {
    fn next(&mut self) -> String {
        self.0 += 1;
        format!("block-{}", self.0)
    }
}

fn nodes_to_blocks(
    nodes: Vec<Node>,
    ids: &mut Ids,
    diagnostics: &mut Vec<String>,
) -> Vec<DocumentBlock> {
    let mut blocks = Vec::new();
    let mut inline_run = Vec::new();
    for node in nodes {
        if matches!(
            node,
            Node::Text(_)
                | Node::Strong(_)
                | Node::Emphasis(_)
                | Node::Strikethrough(_)
                | Node::Link(..)
                | Node::Image(..)
                | Node::InlineCode(_)
                | Node::SoftBreak
                | Node::HardBreak
        ) {
            inline_run.push(node);
            continue;
        }
        flush_inline_run(&mut inline_run, &mut blocks, ids);
        match node {
            Node::Paragraph(children) => {
                let content = nodes_to_inline(children);
                if !content.is_empty() {
                    let mut text_buf = String::new();
                    for node in &content {
                        if let InlineNode::Text { text } = node {
                            text_buf.push_str(text);
                        }
                    }
                    let trimmed = text_buf.trim();
                    let candidate = if let Some(rest) = trimmed.strip_prefix("vak\n").or_else(|| trimmed.strip_prefix("vak ")) {
                        rest.trim()
                    } else {
                        trimmed
                    };
                    if candidate.starts_with('{') && candidate.contains("\"semantic_type\"") {
                        if let Ok(output) = crate::skills::parse_fragment(candidate) {
                            let fallback_markdown = crate::skills::structured_markdown(&output);
                            blocks.push(DocumentBlock::Structured {
                                id: ids.next(),
                                output,
                                fallback_markdown,
                            });
                            continue;
                        }
                    }
                    blocks.push(DocumentBlock::Paragraph {
                        id: ids.next(),
                        content,
                    });
                }
            }
            Node::Heading(level, children) => blocks.push(DocumentBlock::Heading {
                id: ids.next(),
                level,
                content: nodes_to_inline(children),
            }),
            Node::BlockQuote(children) => blocks.push(DocumentBlock::Quote {
                id: ids.next(),
                blocks: nodes_to_blocks(children, ids, diagnostics),
            }),
            Node::CodeBlock(language, content) => {
                if language.as_deref() == Some("vak") || content.contains("\"semantic_type\"") {
                    let trimmed = content.trim();
                    let candidate = if let Some(rest) = trimmed.strip_prefix("vak\n").or_else(|| trimmed.strip_prefix("vak ")) {
                        rest.trim()
                    } else {
                        trimmed
                    };
                    match crate::skills::parse_fragment(candidate) {
                        Ok(output) => {
                            let fallback_markdown = crate::skills::structured_markdown(&output);
                            blocks.push(DocumentBlock::Structured {
                                id: ids.next(),
                                output,
                                fallback_markdown,
                            });
                            continue;
                        }
                        Err(error) => {
                            if language.as_deref() == Some("vak") {
                                diagnostics
                                    .push(format!("Structured block parsing suppressed: {error}"));
                                continue;
                            }
                        }
                    }
                }
                if language.as_deref() == Some("mermaid") {
                    blocks.push(DocumentBlock::Diagram {
                        id: ids.next(),
                        fallback_markdown: content.clone(),
                        source: content,
                    });
                    continue;
                }
                let is_diff = language.as_deref() == Some("diff");
                blocks.push(if is_diff {
                    DocumentBlock::Diff {
                        id: ids.next(),
                        content,
                    }
                } else {
                    DocumentBlock::Code {
                        id: ids.next(),
                        language,
                        filename: None,
                        content,
                    }
                });
            }
            Node::List(start, children) => {
                let items = children
                    .into_iter()
                    .filter_map(|child| match child {
                        Node::Item(item) => Some(nodes_to_blocks(item, ids, diagnostics)),
                        _ => None,
                    })
                    .collect();
                blocks.push(DocumentBlock::List {
                    id: ids.next(),
                    ordered: start.is_some(),
                    start,
                    items,
                });
            }
            Node::Table(alignments, children) => {
                let mut header = Vec::new();
                let mut rows = Vec::new();
                for child in children {
                    match child {
                        Node::TableHead(cells) => header = table_cells(cells),
                        Node::TableRow(cells) => rows.push(table_cells(cells)),
                        _ => {}
                    }
                }
                blocks.push(DocumentBlock::Table {
                    id: ids.next(),
                    alignments,
                    header,
                    rows,
                });
            }
            Node::Rule => blocks.push(DocumentBlock::Rule { id: ids.next() }),
            Node::RawHtml(markdown) => {
                diagnostics.push("raw HTML was preserved as inert text".into());
                blocks.push(DocumentBlock::RawMarkdown {
                    id: ids.next(),
                    markdown,
                    reason: "raw HTML is not mounted by native renderers".into(),
                });
            }
            Node::Unsupported(kind, children) => {
                diagnostics.push(format!("unsupported CommonMark node preserved: {kind}"));
                blocks.push(DocumentBlock::RawMarkdown {
                    id: ids.next(),
                    markdown: inline_text(&nodes_to_inline(children)),
                    reason: format!("unsupported CommonMark node: {kind}"),
                });
            }
            other => {
                let inline = nodes_to_inline(vec![other]);
                if !inline.is_empty() {
                    blocks.push(DocumentBlock::Paragraph {
                        id: ids.next(),
                        content: inline,
                    });
                }
            }
        }
    }
    flush_inline_run(&mut inline_run, &mut blocks, ids);
    blocks
}

fn flush_inline_run(nodes: &mut Vec<Node>, blocks: &mut Vec<DocumentBlock>, ids: &mut Ids) {
    if nodes.is_empty() {
        return;
    }
    let content = nodes_to_inline(std::mem::take(nodes));
    if !content.is_empty() {
        blocks.push(DocumentBlock::Paragraph {
            id: ids.next(),
            content,
        });
    }
}

fn table_cells(nodes: Vec<Node>) -> Vec<Vec<InlineNode>> {
    nodes
        .into_iter()
        .filter_map(|node| match node {
            Node::TableCell(children) => Some(nodes_to_inline(children)),
            _ => None,
        })
        .collect()
}

fn nodes_to_inline(nodes: Vec<Node>) -> Vec<InlineNode> {
    nodes
        .into_iter()
        .filter_map(|node| match node {
            Node::Text(text) => Some(InlineNode::Text { text }),
            Node::InlineCode(code) => Some(InlineNode::Code { code }),
            Node::SoftBreak => Some(InlineNode::SoftBreak),
            Node::HardBreak => Some(InlineNode::HardBreak),
            Node::RawHtml(html) => Some(InlineNode::RawHtml { html }),
            Node::Strong(children) => Some(InlineNode::Strong {
                content: nodes_to_inline(children),
            }),
            Node::Emphasis(children) => Some(InlineNode::Emphasis {
                content: nodes_to_inline(children),
            }),
            Node::Strikethrough(children) => Some(InlineNode::Strikethrough {
                content: nodes_to_inline(children),
            }),
            Node::Link(url, title, children) => Some(InlineNode::Link {
                label: nodes_to_inline(children),
                safe: safe_link(&url),
                url,
                title,
            }),
            Node::Image(url, title, children) => Some(InlineNode::Image {
                alt: inline_text(&nodes_to_inline(children)),
                safe: safe_media(&url),
                url,
                title,
            }),
            Node::Paragraph(children) | Node::TableCell(children) | Node::Item(children) => {
                Some(InlineNode::Text {
                    text: inline_text(&nodes_to_inline(children)),
                })
            }
            _ => None,
        })
        .collect()
}

pub fn inline_text(nodes: &[InlineNode]) -> String {
    let mut output = String::new();
    for node in nodes {
        match node {
            InlineNode::Text { text } => output.push_str(text),
            InlineNode::Strong { content }
            | InlineNode::Emphasis { content }
            | InlineNode::Strikethrough { content } => output.push_str(&inline_text(content)),
            InlineNode::Code { code } => output.push_str(code),
            InlineNode::Link { label, .. } => output.push_str(&inline_text(label)),
            InlineNode::Image { alt, .. } => output.push_str(alt),
            InlineNode::SoftBreak | InlineNode::HardBreak => output.push('\n'),
            InlineNode::RawHtml { html } => output.push_str(html),
        }
    }
    output
}

pub fn safe_link(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
        || lower.starts_with('#')
        || (!lower.contains(':') && !lower.starts_with("//"))
}

fn safe_media(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("data:image/")
        || lower.starts_with('/')
        || (!lower.contains(':') && !lower.starts_with("//"))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        DocumentBlock, InlineNode, OutputContent, OutputTimeline, PRESENTATION_SCHEMA_VERSION,
        compile_markdown, safe_link,
    };
    use std::collections::BTreeMap;

    #[test]
    fn tight_lists_keep_inline_runs_together() {
        let source = "- Stocks include **Axis Bank**, **Adani Ports**, and **HDFC Bank**.";
        let document = compile_markdown(source);
        let DocumentBlock::List { items, .. } = &document.blocks[0] else {
            panic!("expected list");
        };
        assert_eq!(items[0].len(), 1);
        let DocumentBlock::Paragraph { content, .. } = &items[0][0] else {
            panic!("expected paragraph");
        };
        assert_eq!(content.len(), 7);
        assert_eq!(
            super::inline_text(content),
            "Stocks include Axis Bank, Adani Ports, and HDFC Bank."
        );
        assert_eq!(document.source_markdown, source);
    }

    #[test]
    fn adaptive_output_roundtrips_with_fallback_for_replay_and_voice() {
        let mut props = BTreeMap::new();
        props.insert("text".into(), serde_json::json!("Weekend plan"));
        let content = OutputContent::Adaptive {
            tree: vak_presentation::RenderTree {
                schema_version: vak_presentation::RENDER_TREE_SCHEMA_VERSION,
                spec_id: "plan.timeline".into(),
                revision: 1,
                digest: "digest".into(),
                root: vak_presentation::RenderNode {
                    primitive: vak_presentation::Primitive::Title,
                    props,
                    children: Vec::new(),
                },
                accessibility_summary: Some("Weekend plan".into()),
                coverage: vak_presentation::Coverage::default(),
            },
            fallback_text: "Weekend plan\n\nSaturday: travel".into(),
        };
        let encoded = serde_json::to_string(&content).expect("serialize adaptive output");
        let decoded: OutputContent =
            serde_json::from_str(&encoded).expect("replay adaptive output");
        assert_eq!(decoded, content);
        let OutputContent::Adaptive { fallback_text, .. } = decoded else {
            panic!("expected adaptive output");
        };
        assert!(fallback_text.contains("Saturday"));
    }

    #[test]
    fn compiled_adaptive_renderer_keeps_exact_fallback() {
        let mut props = BTreeMap::new();
        props.insert("text".into(), serde_json::json!("Weekend plan"));
        let compiled = vak_presentation::CompiledPresentation::Rich(vak_presentation::RenderTree {
            schema_version: vak_presentation::RENDER_TREE_SCHEMA_VERSION,
            spec_id: "plan.timeline".into(),
            revision: 1,
            digest: "digest".into(),
            root: vak_presentation::RenderNode {
                primitive: vak_presentation::Primitive::Title,
                props,
                children: Vec::new(),
            },
            accessibility_summary: Some("Weekend plan".into()),
            coverage: vak_presentation::Coverage::default(),
        });
        let content =
            OutputContent::from_compiled_adaptive(compiled, "Weekend plan\n\nSaturday: travel")
                .expect("rich compilation should produce adaptive content");
        let OutputContent::Adaptive { fallback_text, .. } = content else {
            panic!("expected adaptive content");
        };
        assert_eq!(fallback_text, "Weekend plan\n\nSaturday: travel");
    }

    #[test]
    fn schema_v2_replay_ignores_future_fields_and_keeps_fallback() {
        // This fixture represents a persisted v2 timeline.  Unknown fields
        // are intentionally tolerated so additive presentation metadata does
        // not make an older desktop/bridge unable to replay a session.
        let fixture = r#"{
          "schema_version": 2,
          "session_id": "session-legacy",
          "items": [{
            "id": "item-1", "timestamp": "2026-01-01T00:00:00Z",
            "turn_id": "turn-1", "role": "assistant", "kind": "message",
            "status": "succeeded",
            "content": {"type":"adaptive", "tree": {
              "schema_version": 1, "spec_id":"plan.timeline", "revision":1,
              "digest":"old-digest", "root":{"primitive":"title","props":{"text":"Plan"},"children":[]},
              "accessibility_summary":"Plan", "coverage":{"rendered_paths":[],"omitted_paths":[]}
            }, "fallback_text":"Plan\n\nSaturday: travel"},
            "fallback_text":"Plan\n\nSaturday: travel", "future_metadata":{"renderer":"new"}
          }], "diagnostics": [], "future_timeline_field": true
        }"#;
        let timeline: OutputTimeline = serde_json::from_str(fixture).expect("v2 replay fixture");
        assert_eq!(timeline.schema_version, PRESENTATION_SCHEMA_VERSION);
        assert_eq!(timeline.items[0].fallback_text, "Plan\n\nSaturday: travel");
        let OutputContent::Adaptive { fallback_text, .. } = &timeline.items[0].content else {
            panic!("expected adaptive replay item");
        };
        assert_eq!(fallback_text, &timeline.items[0].fallback_text);
    }

    #[test]
    fn schema_v2_delivery_packet_keeps_fallback_when_new_fields_arrive() {
        // Delivery packets are also durable upgrade inputs.  The packet's
        // exact Markdown fallback remains authoritative when a newer writer
        // adds fields an older bridge does not know about.
        let fixture = r#"{
          "schema_version": 2,
          "job_id": "job-legacy",
          "target": "webhook:demo",
          "surface": "webhook",
          "kind": "assistant",
          "payload": {"type":"text","value":"Plan"},
          "fallback_markdown": "Plan\n\nSaturday: travel",
          "chunks": ["Plan", "Saturday: travel"],
          "actions": [],
          "coverage": [],
          "diagnostics": [],
          "future_delivery_metadata": {"chunk_format":"rich"}
        }"#;
        let packet: crate::DeliveryPacket =
            serde_json::from_str(fixture).expect("schema-v2 delivery packet");
        assert_eq!(packet.schema_version, 2);
        assert_eq!(packet.fallback_markdown, "Plan\n\nSaturday: travel");
        assert_eq!(packet.chunks.len(), 2);
    }

    #[test]
    fn loose_paragraphs_and_nested_lists_keep_their_boundaries() {
        let document = compile_markdown(
            "- First **paragraph**.\n\n  Second paragraph.\n\n  - Nested [link](https://example.com)\n\n  Last paragraph.",
        );
        let DocumentBlock::List { items, .. } = &document.blocks[0] else {
            panic!("expected list");
        };
        assert_eq!(items[0].len(), 4);
        assert!(matches!(items[0][2], DocumentBlock::List { .. }));
    }

    #[test]
    fn compiles_commonmark_without_losing_source() {
        let source = "# Result\n\nA **strong** [link](https://example.com).\n\n- one\n- two\n\n| a | b |\n|---|---:|\n| 1 | 2 |\n\n```rust\nfn main() {}\n```";
        let document = compile_markdown(source);
        assert_eq!(document.source_markdown, source);
        assert!(matches!(document.blocks[0], DocumentBlock::Heading { .. }));
        assert!(
            document.blocks.iter().any(
                |block| matches!(block, DocumentBlock::List { items, .. } if items.len() == 2)
            )
        );
        assert!(
            document
                .blocks
                .iter()
                .any(|block| matches!(block, DocumentBlock::Table { rows, .. } if rows.len() == 1))
        );
        assert!(document.blocks.iter().any(|block| matches!(block, DocumentBlock::Code { language, .. } if language.as_deref() == Some("rust"))));
    }

    #[test]
    fn unsafe_links_and_html_remain_inert() {
        let document = compile_markdown("[bad](javascript:alert(1))\n\n<script>x</script>");
        let DocumentBlock::Paragraph { content, .. } = &document.blocks[0] else {
            panic!("first block should be a paragraph");
        };
        assert!(matches!(&content[0], InlineNode::Link { safe: false, .. }));
        assert!(!safe_link("javascript:alert(1)"));
        assert!(!document.diagnostics.is_empty());
    }

    #[test]
    fn fenced_diff_receives_a_native_block() {
        let document = compile_markdown("```diff\n-old\n+new\n```");
        assert!(matches!(document.blocks[0], DocumentBlock::Diff { .. }));
    }

    #[test]
    fn specialized_renderer_golden_fixture_preserves_source_and_boundaries() {
        // This compact fixture intentionally exercises the presentation
        // primitives that have historically had specialized desktop views.
        // The source remains the canonical fallback: adding a native view
        // must not alter what a voice or constrained channel receives.
        let source = "# Release review\n\n- [x] Diff reviewed\n- [ ] Tests expanded\n\n| Check | Result |\n| --- | --- |\n| Tests | 12 passed |\n\n```diff\n-old\n+new\n```\n\n```bash\n$ cargo test\n```\n\n> **Approval:** publish the artifact\n\n[Report](https://example.com/report)\n\n![Chart](https://example.com/chart.png)";
        let document = compile_markdown(source);
        assert_eq!(document.source_markdown, source);
        assert!(
            document
                .blocks
                .iter()
                .any(|block| matches!(block, DocumentBlock::Diff { .. }))
        );
        assert!(
            document
                .blocks
                .iter()
                .any(|block| matches!(block, DocumentBlock::Table { rows, .. } if rows.len() == 1))
        );
        assert!(document.blocks.iter().any(|block| matches!(block, DocumentBlock::Code { language, .. } if language.as_deref() == Some("bash"))));
        assert!(document.blocks.iter().any(|block| matches!(block, DocumentBlock::Paragraph { content, .. } if content.iter().any(|node| matches!(node, InlineNode::Link { safe: true, .. })))));

        let encoded = serde_json::to_string(&document).expect("serialize specialized fixture");
        let replayed: super::PresentationDocument =
            serde_json::from_str(&encoded).expect("replay specialized fixture");
        assert_eq!(replayed, document);
        assert!(replayed.coverage.iter().all(|entry| {
            matches!(
                entry.disposition,
                super::DocumentCoverageDisposition::Native
            )
        }));
    }

    #[test]
    fn specialized_output_envelopes_roundtrip_without_losing_fallback() {
        let fallback = "Publish the report after approval.";
        let outputs = [
            OutputContent::Approval {
                request_id: "approval-1".into(),
                tool: "publish".into(),
                args_json: "{}".into(),
                reason: "User confirmation required".into(),
                expires_at: None,
            },
            OutputContent::Artifact {
                artifact: super::ArtifactRef {
                    name: "report.pdf".into(),
                    path: Some(".vak/scratch/report.pdf".into()),
                    media_type: Some("application/pdf".into()),
                    description: Some("Generated report".into()),
                },
            },
            OutputContent::Document {
                document: compile_markdown(fallback),
            },
        ];
        for output in outputs {
            let encoded = serde_json::to_string(&output).expect("serialize output envelope");
            let replayed: OutputContent =
                serde_json::from_str(&encoded).expect("replay output envelope");
            assert_eq!(replayed, output);
        }
        assert_eq!(fallback, "Publish the report after approval.");
    }

    #[test]
    fn mermaid_is_preserved_as_a_safe_diagram_source() {
        let document = compile_markdown("```mermaid\ngraph TD; A-->B;\n```");
        assert!(
            matches!(document.blocks[0], DocumentBlock::Diagram { ref source, .. } if source.contains("A-->B"))
        );
        assert_eq!(
            document.source_markdown,
            "```mermaid\ngraph TD; A-->B;\n```"
        );
    }

    #[test]
    fn malformed_input_roundtrips_with_explicit_coverage() {
        let source = "## Unclosed **strong\n\n<div onclick=\"bad()\">literal</div>";
        let document = compile_markdown(source);
        let encoded = serde_json::to_string(&document).expect("serialize presentation");
        let decoded: super::PresentationDocument =
            serde_json::from_str(&encoded).expect("deserialize presentation");
        assert_eq!(decoded.source_markdown, source);
        assert!(!decoded.coverage.is_empty());
        assert_eq!(decoded, document);
        assert!(!decoded.diagnostics.is_empty());
    }
}
