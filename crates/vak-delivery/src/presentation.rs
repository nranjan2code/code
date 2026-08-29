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
    pub content: OutputContent,
    #[serde(default)]
    pub provenance: Option<OutputProvenance>,
    #[serde(default)]
    pub actions: Vec<crate::DeliveryAction>,
    pub fallback_text: String,
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
}

impl OutputTimeline {
    pub fn empty(session_id: impl Into<String>) -> Self {
        Self {
            schema_version: PRESENTATION_SCHEMA_VERSION,
            session_id: session_id.into(),
            cursor: None,
            items: Vec::new(),
            diagnostics: Vec::new(),
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
    },
    ItemReplaced {
        item: OutputItem,
    },
    ItemCompleted {
        item_id: String,
        status: OutputStatus,
    },
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
    let source_markdown = source.into();
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
    for node in nodes {
        match node {
            Node::Paragraph(children) => {
                let content = nodes_to_inline(children);
                if !content.is_empty() {
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
    blocks
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
    use super::{DocumentBlock, InlineNode, compile_markdown, safe_link};

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
