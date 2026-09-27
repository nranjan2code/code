//! L2 read projection: every page as anchored lines with labels, the
//! outline, the document information, and the security inspection
//! (docs/design/77-pdf-documents.md).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::{Range, RangeInclusive};

use serde::Serialize;

use crate::content::{self, FontCache, Interpreter};
use crate::file::File;
use crate::object::{Dict, NULL, Object, Stream};
use crate::text::text_string;
use crate::{Error, Limits};

/// Longest text one rendered line carries; a longer line continues on
/// further lines that name the same anchor and their part, so no text is
/// dropped and no line can outgrow a page.
pub const MAX_LINE_CHARS: usize = 4_000;

/// US Letter, the page box assumed when a page declares none.
const LETTER: [f64; 4] = [0.0, 0.0, 612.0, 792.0];

const MAX_TREE_DEPTH: usize = 64;
const MAX_PAGE_COUNT: usize = 1_000_000;
const MAX_OUTLINE_DEPTH: usize = 32;
const MAX_NAMES: usize = 50_000;
const MAX_LINKS: usize = 1_000;
const MAX_LINK_CHARS: usize = 2_000;
const MAX_LISTED_FILES: usize = 50;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Info {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub producer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
}

/// One line of a page: `page:<n>/line:<m>`, both counted from 1 as a
/// viewer counts pages. Labels say what a reader of the page does not see
/// as body text: `invisible`, `white`, `tiny` and `off-page` text, a
/// `comment`, a `form field`, a `hidden` annotation.
#[derive(Debug, Clone, Serialize)]
pub struct Line {
    pub anchor: String,
    pub text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Page {
    pub number: usize,
    pub lines: Vec<Line>,
    pub images: usize,
    /// Why this page's content could not be read, when it could not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_read: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Bookmark {
    pub level: usize,
    pub title: String,
    pub page: Option<usize>,
}

/// A link target, recorded and never followed.
#[derive(Debug, Clone, Serialize)]
pub struct ExternalLink {
    pub page: usize,
    pub target: String,
}

/// What the file could do if a viewer let it, and what state it is in.
/// Nothing counted here is ever run, opened, followed or verified.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Inspection {
    pub javascript: usize,
    pub automatic_actions: usize,
    pub launch_actions: usize,
    pub submit_actions: usize,
    pub remote_actions: usize,
    pub embedded_files: Vec<String>,
    pub xfa: bool,
    pub form_fields: usize,
    pub signatures: usize,
    pub rich_media: usize,
    pub external_links: Vec<ExternalLink>,
    /// Saved revisions: more than one means the file was changed by
    /// incremental updates after it was first written.
    pub revisions: usize,
    pub rebuilt: bool,
    pub damaged_objects: usize,
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

impl Inspection {
    pub fn flags(&self) -> Vec<String> {
        let mut flags = Vec::new();
        if self.javascript > 0 {
            flags.push(format!(
                "{} (never run)",
                plural(self.javascript, "JavaScript action")
            ));
        }
        if self.automatic_actions > 0 {
            flags.push(format!(
                "{} that run on opening or on events (never run)",
                plural(self.automatic_actions, "automatic action")
            ));
        }
        if self.launch_actions > 0 {
            flags.push(format!(
                "{} that would start a program (never run)",
                plural(self.launch_actions, "launch action")
            ));
        }
        if self.submit_actions > 0 {
            flags.push(format!(
                "{} (never sent)",
                plural(self.submit_actions, "form submit action")
            ));
        }
        if self.remote_actions > 0 {
            flags.push(format!(
                "{} that open or import another file (never followed)",
                plural(self.remote_actions, "action")
            ));
        }
        if !self.embedded_files.is_empty() {
            flags.push(format!(
                "{}: {} (never opened)",
                plural(self.embedded_files.len(), "embedded file"),
                self.embedded_files.join(", ")
            ));
        }
        if self.xfa {
            flags.push("an XFA form (never rendered)".into());
        }
        if self.signatures > 0 {
            flags.push(format!(
                "{} (signatures are not verified)",
                plural(self.signatures, "signature field")
            ));
        }
        if self.form_fields > 0 {
            flags.push(plural(self.form_fields, "form field"));
        }
        if self.rich_media > 0 {
            flags.push(format!(
                "{} (never played)",
                plural(self.rich_media, "media, 3D or screen annotation")
            ));
        }
        if !self.external_links.is_empty() {
            flags.push(format!(
                "{} (never followed)",
                plural(self.external_links.len(), "external link")
            ));
        }
        if self.revisions > 1 {
            flags.push(format!(
                "{} (changed after it was first saved; the latest is read)",
                plural(self.revisions, "saved revision")
            ));
        }
        if self.rebuilt {
            flags.push("damaged cross-reference table (objects were found by scanning)".into());
        }
        if self.damaged_objects > 0 {
            flags.push(format!(
                "{} skipped",
                plural(self.damaged_objects, "damaged object")
            ));
        }
        flags
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Document {
    pub version: String,
    pub info: Info,
    /// Pages in the file; `pages` holds those read, at most the page limit.
    pub page_count: usize,
    pub pages: Vec<Page>,
    pub outline: Vec<Bookmark>,
    pub inspection: Inspection,
    pub not_read: Vec<String>,
}

/// Reads a PDF: bounded (`limits`), read-only, and nothing in it executed
/// or followed. A defect in this reader on some input is reported as a
/// malformed file, never as a crash of the process.
pub fn read(bytes: &[u8], limits: Limits) -> Result<Document, Error> {
    if bytes.len() as u64 > limits.max_file_bytes {
        return Err(Error::TooLarge {
            bytes: bytes.len() as u64,
            limit: limits.max_file_bytes,
        });
    }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read_file(bytes, limits)))
        .unwrap_or_else(|_| Err(Error::Malformed("the reader failed on it".into())))
}

fn read_file(bytes: &[u8], limits: Limits) -> Result<Document, Error> {
    let file = File::open(bytes, limits)?;
    let catalog = file
        .catalog()
        .ok_or_else(|| Error::Malformed("it has no document catalog".into()))?;
    let tree = PageTree::walk(&file, catalog, &limits);
    if tree.count == 0 {
        return Err(Error::NoPages);
    }
    let mut inspection = inspect(&file, catalog);
    let mut fonts = FontCache::new();
    let mut pages = Vec::with_capacity(tree.pages.len());
    let mut failed: Vec<usize> = Vec::new();
    let mut failure: Option<String> = None;
    let mut undecodable: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let mut scanned: Vec<usize> = Vec::new();
    let mut truncated: Vec<usize> = Vec::new();
    let mut unmapped = 0usize;
    for (index, node) in tree.pages.iter().enumerate() {
        let number = index + 1;
        let mut data = Vec::new();
        let mut page_failure = None;
        for stream in contents(&file, node.dict) {
            match file.decode(stream) {
                Ok(decoded) => {
                    data.extend_from_slice(&decoded);
                    data.push(b'\n');
                }
                Err(error) => {
                    page_failure = Some(error);
                    break;
                }
            }
        }
        let mut interpreter = Interpreter::new(&file, &limits, node.page_box, &mut fonts);
        interpreter.run_page(&data, node.resources);
        let text = interpreter.finish();
        let page_failure = page_failure.or_else(|| text.errors.first().cloned());
        if let Some(error) = &page_failure {
            failed.push(number);
            failure.get_or_insert_with(|| error.clone());
        }
        for font in &text.undecodable {
            undecodable.entry(font.clone()).or_default().insert(number);
        }
        if text.truncated {
            truncated.push(number);
        }
        unmapped += text.unmapped;
        let mut lines: Vec<(String, Vec<String>)> = text
            .lines
            .into_iter()
            .map(|(text, flags)| (text, content::label_names(flags)))
            .collect();
        let body_lines = lines.len();
        annotations(
            &file,
            node.dict,
            number,
            &limits,
            &mut inspection,
            &mut lines,
        );
        if body_lines == 0 && text.images > 0 && page_failure.is_none() {
            scanned.push(number);
        }
        pages.push(Page {
            number,
            lines: lines
                .into_iter()
                .enumerate()
                .map(|(line, (text, labels))| Line {
                    anchor: format!("page:{number}/line:{}", line + 1),
                    text,
                    labels,
                })
                .collect(),
            images: text.images,
            not_read: page_failure,
        });
    }
    let mut not_read = Vec::new();
    if tree.count > tree.pages.len() {
        not_read.push(format!(
            "pages {}–{} (over the {}-page limit)",
            tree.pages.len() + 1,
            tree.count,
            limits.max_pages
        ));
    }
    if let Some(failure) = failure {
        not_read.push(format!("the content of {}: {failure}", page_list(&failed)));
    }
    if !undecodable.is_empty() {
        let pages: BTreeSet<usize> = undecodable.values().flatten().copied().collect();
        not_read.push(format!(
            "text in {} that give no way to turn glyphs into letters ({}) on {}",
            plural(undecodable.len(), "font"),
            undecodable.keys().cloned().collect::<Vec<_>>().join(", "),
            page_list(&pages.into_iter().collect::<Vec<_>>())
        ));
    }
    if !scanned.is_empty() {
        not_read.push(format!(
            "{} with images and no text layer, likely a scan (there is no OCR)",
            page_list(&scanned)
        ));
    }
    if !truncated.is_empty() {
        not_read.push(format!(
            "the rest of {} (over the line or operator limit)",
            page_list(&truncated)
        ));
    }
    if unmapped > 0 {
        not_read.push(format!(
            "{} with no Unicode mapping, shown as \u{FFFD}",
            plural(unmapped, "character")
        ));
    }
    if inspection.xfa {
        not_read.push("the XFA form (only the PDF pages were read)".into());
    }
    let version = catalog
        .name(b"Version")
        .map(|version| String::from_utf8_lossy(version).into_owned())
        .filter(|version| version.as_str() > file.version.as_str())
        .unwrap_or_else(|| file.version.clone());
    Ok(Document {
        version,
        info: info(&file),
        page_count: tree.count,
        outline: outline(&file, catalog, &tree.numbers, &limits),
        pages,
        inspection,
        not_read,
    })
}

/// `page 3`, or `pages 1–3, 5`.
fn page_list(pages: &[usize]) -> String {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for &page in pages {
        match ranges.last_mut() {
            Some((_, end)) if *end + 1 == page => *end = page,
            _ => ranges.push((page, page)),
        }
    }
    let text = ranges
        .iter()
        .map(|(start, end)| {
            if start == end {
                start.to_string()
            } else {
                format!("{start}–{end}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    if pages.len() == 1 {
        format!("page {text}")
    } else {
        format!("pages {text}")
    }
}

fn clean(text: String) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Document {
    pub fn title(&self) -> Option<&str> {
        self.info.title.as_deref()
    }

    /// The document as anchored lines, `[anchor] text  ⟨labels⟩`, and one
    /// `[page:n] ⟨…⟩` line for a page with no text.
    pub fn lines(&self) -> Vec<String> {
        self.pages.iter().flat_map(render_page).collect()
    }

    /// Anchored lines for the pages numbered in `pages`.
    pub fn lines_of(&self, pages: RangeInclusive<usize>) -> Vec<String> {
        self.pages
            .iter()
            .filter(|page| pages.contains(&page.number))
            .flat_map(render_page)
            .collect()
    }

    /// The bookmarks with the page each opens, or when there are none, the
    /// pages with the first line of each.
    pub fn outline_lines(&self) -> Vec<String> {
        if !self.outline.is_empty() {
            return self
                .outline
                .iter()
                .map(|bookmark| {
                    let indent = "  ".repeat(bookmark.level.saturating_sub(1).min(8));
                    match bookmark.page {
                        Some(page) => format!("{indent}- [page:{page}] {}", bookmark.title),
                        None => format!("{indent}- {}", bookmark.title),
                    }
                })
                .collect();
        }
        self.pages
            .iter()
            .map(|page| {
                let first = page
                    .lines
                    .iter()
                    .find(|line| line.labels.is_empty())
                    .or(page.lines.first())
                    .map(|line| line.text.chars().take(80).collect::<String>())
                    .unwrap_or_else(|| "⟨no text⟩".into());
                format!("- [page:{}] {first}", page.number)
            })
            .collect()
    }

    /// The pages a section names: `page:3` (or an anchor on it), `3`,
    /// `3-5`, `pages 3-5`, or a bookmark by exact title then title
    /// fragment, which runs to the next bookmark at its level or above.
    pub fn section(&self, wanted: &str) -> Option<RangeInclusive<usize>> {
        let wanted = wanted.trim().trim_start_matches('#').trim().to_lowercase();
        let numbers = wanted
            .trim_start_matches("pages")
            .trim_start_matches("page")
            .trim_start_matches(':')
            .trim();
        let numbers = numbers.split('/').next().unwrap_or(numbers);
        let page = |text: &str| {
            text.trim()
                .parse::<usize>()
                .ok()
                .filter(|page| (1..=self.page_count).contains(page))
        };
        if let Some((first, last)) = numbers.split_once(['-', '–'])
            && let (Some(first), Some(last)) = (page(first), page(last))
            && first <= last
        {
            return Some(first..=last);
        }
        if let Some(page) = page(numbers) {
            return Some(page..=page);
        }
        let index = self
            .outline
            .iter()
            .position(|bookmark| bookmark.title.to_lowercase() == wanted)
            .or_else(|| {
                self.outline
                    .iter()
                    .position(|bookmark| bookmark.title.to_lowercase().contains(&wanted))
            })?;
        let bookmark = self.outline.get(index)?;
        let start = bookmark.page?;
        let end = self
            .outline
            .get(index + 1..)
            .unwrap_or_default()
            .iter()
            .find(|next| next.level <= bookmark.level)
            .and_then(|next| next.page)
            .map(|next| next.saturating_sub(1).max(start))
            .unwrap_or(self.page_count);
        Some(start..=end)
    }

    /// The page an anchor names, when that page was read.
    pub fn locate(&self, anchor: &str) -> Option<usize> {
        if !crate::is_anchor(anchor) {
            return None;
        }
        let page = anchor
            .strip_prefix("page:")?
            .split('/')
            .next()?
            .parse::<usize>()
            .ok()?;
        self.pages
            .iter()
            .any(|read| read.number == page)
            .then_some(page)
    }

    pub fn stats(&self) -> Vec<(&'static str, usize)> {
        let comments = self
            .pages
            .iter()
            .flat_map(|page| &page.lines)
            .filter(|line| line.labels.iter().any(|label| label.starts_with("comment")))
            .count();
        vec![
            ("pages", self.page_count),
            (
                "pages with text",
                self.pages
                    .iter()
                    .filter(|page| page.lines.iter().any(|line| line.labels.is_empty()))
                    .count(),
            ),
            ("images", self.pages.iter().map(|page| page.images).sum()),
            ("links", self.inspection.external_links.len()),
            ("comments", comments),
            ("form fields", self.inspection.form_fields),
            ("bookmarks", self.outline.len()),
        ]
    }
}

fn render_page(page: &Page) -> Vec<String> {
    if page.lines.is_empty() {
        let why = match (&page.not_read, page.images) {
            (Some(reason), _) => format!("not read: {reason}"),
            (None, 0) => "no text".to_string(),
            (None, images) => format!(
                "no text, {}: likely a scan, which is not read",
                plural(images, "image")
            ),
        };
        return vec![format!("[page:{}] ⟨{why}⟩", page.number)];
    }
    page.lines.iter().flat_map(render_line).collect()
}

fn render_line(line: &Line) -> Vec<String> {
    let characters: Vec<char> = line.text.chars().collect();
    let chunks: Vec<String> = if characters.len() <= MAX_LINE_CHARS {
        vec![line.text.clone()]
    } else {
        characters
            .chunks(MAX_LINE_CHARS)
            .map(|chunk| chunk.iter().collect())
            .collect()
    };
    let parts = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, text)| {
            let mut out = if parts == 1 {
                format!("[{}] ", line.anchor)
            } else {
                format!("[{} ⟨part {} of {parts}⟩] ", line.anchor, index + 1)
            };
            out.push_str(&text);
            if !line.labels.is_empty() {
                out.push_str("  ⟨");
                out.push_str(&line.labels.join("; "));
                out.push('⟩');
            }
            out
        })
        .collect()
}

struct PageNode<'f> {
    dict: &'f Dict,
    resources: Option<&'f Dict>,
    page_box: [f64; 4],
}

#[derive(Clone, Copy, Default)]
struct Inherited<'f> {
    resources: Option<&'f Dict>,
    media: Option<[f64; 4]>,
    crop: Option<[f64; 4]>,
}

/// The page tree in order, with inherited resources and page boxes; a
/// node reached twice (a cycle) is visited once.
struct PageTree<'f> {
    pages: Vec<PageNode<'f>>,
    count: usize,
    /// Page object number to page number, for destinations.
    numbers: HashMap<u32, usize>,
}

impl<'f> PageTree<'f> {
    fn walk(file: &'f File<'_>, catalog: &'f Dict, limits: &Limits) -> Self {
        let mut tree = PageTree {
            pages: Vec::new(),
            count: 0,
            numbers: HashMap::new(),
        };
        let mut seen = HashSet::new();
        if let Some(root) = catalog.get(b"Pages") {
            tree.visit(file, root, Inherited::default(), 0, &mut seen, limits);
        }
        tree
    }

    fn visit(
        &mut self,
        file: &'f File<'_>,
        node: &'f Object,
        inherited: Inherited<'f>,
        depth: usize,
        seen: &mut HashSet<u32>,
        limits: &Limits,
    ) {
        if depth > MAX_TREE_DEPTH || self.count >= MAX_PAGE_COUNT {
            return;
        }
        let number = node.as_reference().map(|(number, _)| number);
        if let Some(number) = number
            && !seen.insert(number)
        {
            return;
        }
        let Some(dict) = file.dict(node) else {
            return;
        };
        let inherited = Inherited {
            resources: file
                .dict(file.lookup(dict, b"Resources"))
                .or(inherited.resources),
            media: rectangle(file, dict, b"MediaBox").or(inherited.media),
            crop: rectangle(file, dict, b"CropBox").or(inherited.crop),
        };
        let kids = file.lookup(dict, b"Kids").as_array();
        let leaf = match dict.name(b"Type") {
            Some(b"Page") => true,
            Some(b"Pages") => false,
            _ => kids.is_none(),
        };
        if leaf {
            self.count += 1;
            if let Some(number) = number {
                self.numbers.insert(number, self.count);
            }
            if self.pages.len() < limits.max_pages {
                self.pages.push(PageNode {
                    dict,
                    resources: inherited.resources,
                    page_box: inherited.crop.or(inherited.media).unwrap_or(LETTER),
                });
            }
            return;
        }
        for kid in kids.unwrap_or_default() {
            self.visit(file, kid, inherited, depth + 1, seen, limits);
        }
    }
}

fn rectangle(file: &File<'_>, dict: &Dict, key: &[u8]) -> Option<[f64; 4]> {
    let values: Vec<f64> = file
        .lookup(dict, key)
        .as_array()?
        .iter()
        .filter_map(|value| file.resolve(value).as_f64())
        .collect();
    let [a, b, c, d] = values[..] else {
        return None;
    };
    Some([a.min(c), b.min(d), a.max(c), b.max(d)])
}

fn contents<'f>(file: &'f File<'_>, page: &'f Dict) -> Vec<&'f Stream> {
    match file.lookup(page, b"Contents") {
        Object::Stream(stream) => vec![stream],
        Object::Array(items) => items
            .iter()
            .filter_map(|item| file.resolve(item).as_stream())
            .collect(),
        _ => Vec::new(),
    }
}

fn annotations(
    file: &File<'_>,
    page: &Dict,
    number: usize,
    limits: &Limits,
    inspection: &mut Inspection,
    lines: &mut Vec<(String, Vec<String>)>,
) {
    let Some(annotations) = file.lookup(page, b"Annots").as_array() else {
        return;
    };
    for entry in annotations.iter().take(limits.max_annotations) {
        let Some(annotation) = file.dict(entry) else {
            continue;
        };
        let hidden = file
            .lookup(annotation, b"F")
            .as_i64()
            .is_some_and(|flags| flags & (2 | 32) != 0);
        let mut labels = Vec::new();
        let text = match annotation.name(b"Subtype") {
            Some(b"Link") => {
                if let Some(target) = link_target(file, annotation)
                    && inspection.external_links.len() < MAX_LINKS
                {
                    inspection.external_links.push(ExternalLink {
                        page: number,
                        target,
                    });
                }
                continue;
            }
            Some(b"Popup") => continue,
            Some(b"Widget") => {
                let Some((name, value)) = field_value(file, annotation) else {
                    continue;
                };
                labels.push("form field".to_string());
                format!("{name}: {value}")
            }
            _ => {
                let contents = file
                    .lookup(annotation, b"Contents")
                    .as_string()
                    .map(text_string)
                    .map(clean)
                    .unwrap_or_default();
                if contents.is_empty() {
                    continue;
                }
                let author = file
                    .lookup(annotation, b"T")
                    .as_string()
                    .map(text_string)
                    .map(clean)
                    .filter(|author| !author.is_empty());
                labels.push(match author {
                    Some(author) => format!("comment by {author}"),
                    None => "comment".to_string(),
                });
                contents
            }
        };
        if hidden {
            labels.push("hidden".to_string());
        }
        lines.push((text, labels));
    }
}

fn link_target(file: &File<'_>, annotation: &Dict) -> Option<String> {
    let action = file.dict(file.lookup(annotation, b"A"))?;
    let target = match action.name(b"S")? {
        b"URI" => String::from_utf8_lossy(file.lookup(action, b"URI").as_string()?).into_owned(),
        b"Launch" => format!("launch {}", file_name(file, file.lookup(action, b"F"))?),
        b"GoToR" | b"GoToE" => format!(
            "file {}",
            file_name(file, file.lookup(action, b"F")).unwrap_or_else(|| "(embedded)".into())
        ),
        _ => return None,
    };
    Some(
        target
            .chars()
            .filter(|character| !character.is_control())
            .take(MAX_LINK_CHARS)
            .collect(),
    )
}

fn file_name(file: &File<'_>, spec: &Object) -> Option<String> {
    let name = match spec {
        Object::String(bytes) => text_string(bytes),
        Object::Dict(dict) => text_string(
            file.lookup(dict, b"UF")
                .as_string()
                .or_else(|| file.lookup(dict, b"F").as_string())?,
        ),
        _ => return None,
    };
    let name = clean(name);
    (!name.is_empty()).then(|| name.chars().take(200).collect())
}

fn inherited<'f>(
    file: &'f File<'_>,
    widget: &'f Dict,
    parent: Option<&'f Dict>,
    key: &[u8],
) -> &'f Object {
    match file.lookup(widget, key) {
        Object::Null => parent
            .map(|parent| file.lookup(parent, key))
            .unwrap_or(&NULL),
        found => found,
    }
}

fn field_value(file: &File<'_>, widget: &Dict) -> Option<(String, String)> {
    let parent = file.dict(file.lookup(widget, b"Parent"));
    let name = inherited(file, widget, parent, b"T")
        .as_string()
        .map(text_string)
        .map(clean)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "field".into());
    let value = match inherited(file, widget, parent, b"V") {
        Object::String(bytes) => text_string(bytes),
        Object::Name(name) => String::from_utf8_lossy(name).into_owned(),
        Object::Array(items) => items
            .iter()
            .filter_map(|item| file.resolve(item).as_string())
            .map(text_string)
            .collect::<Vec<_>>()
            .join(", "),
        _ => return None,
    };
    let value = clean(value);
    (!value.is_empty() && value != "Off").then_some((name, value))
}

fn inspect(file: &File<'_>, catalog: &Dict) -> Inspection {
    let mut inspection = Inspection::default();
    let mut embedded = BTreeSet::new();
    let mut linearized = false;
    for object in file.objects() {
        scan(
            file,
            object,
            0,
            &mut inspection,
            &mut embedded,
            &mut linearized,
        );
    }
    if catalog
        .get(b"OpenAction")
        .is_some_and(|action| file.resolve(action).as_dict().is_some())
    {
        inspection.automatic_actions += 1;
    }
    inspection.xfa = file
        .dict(file.lookup(catalog, b"AcroForm"))
        .is_some_and(|form| form.has(b"XFA"));
    inspection.embedded_files = embedded.into_iter().take(MAX_LISTED_FILES).collect();
    inspection.revisions = file.revisions.saturating_sub(usize::from(linearized));
    inspection.rebuilt = file.rebuilt;
    inspection.damaged_objects = file.damaged;
    inspection
}

fn scan(
    file: &File<'_>,
    object: &Object,
    depth: usize,
    inspection: &mut Inspection,
    embedded: &mut BTreeSet<String>,
    linearized: &mut bool,
) {
    if depth > 16 {
        return;
    }
    let dict = match object {
        Object::Dict(dict) => dict,
        Object::Stream(stream) => &stream.dict,
        Object::Array(items) => {
            for item in items {
                if matches!(item, Object::Dict(_) | Object::Array(_)) {
                    scan(file, item, depth + 1, inspection, embedded, linearized);
                }
            }
            return;
        }
        _ => return,
    };
    match dict.name(b"S") {
        Some(b"JavaScript") => inspection.javascript += 1,
        Some(b"Launch") => inspection.launch_actions += 1,
        Some(b"SubmitForm") => inspection.submit_actions += 1,
        Some(b"ImportData" | b"GoToR" | b"GoToE") => inspection.remote_actions += 1,
        _ => {
            if dict.has(b"JS") {
                inspection.javascript += 1;
            }
        }
    }
    if dict.has(b"AA") {
        inspection.automatic_actions += 1;
    }
    if dict.has(b"EF") {
        let name = file_name(file, object).unwrap_or_else(|| "unnamed".into());
        embedded.insert(name);
    }
    match dict.name(b"FT") {
        Some(b"Sig") => {
            inspection.signatures += 1;
            inspection.form_fields += 1;
        }
        Some(_) => inspection.form_fields += 1,
        None => {}
    }
    if matches!(
        dict.name(b"Subtype"),
        Some(b"RichMedia" | b"3D" | b"Movie" | b"Sound" | b"Screen")
    ) {
        inspection.rich_media += 1;
    }
    if dict.has(b"Linearized") {
        *linearized = true;
    }
    for (_, value) in dict.iter() {
        if matches!(value, Object::Dict(_) | Object::Array(_)) {
            scan(file, value, depth + 1, inspection, embedded, linearized);
        }
    }
}

fn info(file: &File<'_>) -> Info {
    let Some(dict) = file.dict(file.lookup(&file.trailer, b"Info")) else {
        return Info::default();
    };
    let text = |key: &[u8]| {
        file.lookup(dict, key)
            .as_string()
            .map(text_string)
            .map(clean)
            .filter(|text| !text.is_empty())
    };
    Info {
        title: text(b"Title"),
        author: text(b"Author"),
        subject: text(b"Subject"),
        keywords: text(b"Keywords"),
        creator: text(b"Creator"),
        producer: text(b"Producer"),
        created: text(b"CreationDate").map(date),
        modified: text(b"ModDate").map(date),
    }
}

/// `D:YYYYMMDDHHmmSS…` as `YYYY-MM-DD HH:mm`, or the text as it is.
fn date(raw: String) -> String {
    let digits: String = raw
        .trim_start_matches("D:")
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let part = |range: Range<usize>| digits.get(range);
    match (
        part(0..4),
        part(4..6),
        part(6..8),
        part(8..10),
        part(10..12),
    ) {
        (Some(year), Some(month), Some(day), Some(hour), Some(minute)) => {
            format!("{year}-{month}-{day} {hour}:{minute}")
        }
        (Some(year), Some(month), Some(day), _, _) => format!("{year}-{month}-{day}"),
        _ => raw,
    }
}

fn outline(
    file: &File<'_>,
    catalog: &Dict,
    numbers: &HashMap<u32, usize>,
    limits: &Limits,
) -> Vec<Bookmark> {
    let Some(root) = file.dict(file.lookup(catalog, b"Outlines")) else {
        return Vec::new();
    };
    let names = named_destinations(file, catalog);
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut walk = Outline {
        file,
        names: &names,
        numbers,
        limits,
        seen: &mut seen,
        out: &mut out,
    };
    walk.items(root.get(b"First"), 1);
    out
}

struct Outline<'f, 'a, 'w> {
    file: &'f File<'a>,
    names: &'w HashMap<Vec<u8>, &'f Object>,
    numbers: &'w HashMap<u32, usize>,
    limits: &'w Limits,
    seen: &'w mut HashSet<u32>,
    out: &'w mut Vec<Bookmark>,
}

impl<'f> Outline<'f, '_, '_> {
    fn items(&mut self, first: Option<&'f Object>, level: usize) {
        let file = self.file;
        let mut node = first;
        let mut steps = 0usize;
        while let Some(current) = node {
            steps += 1;
            if self.out.len() >= self.limits.max_bookmarks
                || level > MAX_OUTLINE_DEPTH
                || steps > self.limits.max_bookmarks
            {
                return;
            }
            if let Some((number, _)) = current.as_reference()
                && !self.seen.insert(number)
            {
                return;
            }
            let Some(item) = file.dict(current) else {
                return;
            };
            let title = file
                .lookup(item, b"Title")
                .as_string()
                .map(text_string)
                .map(clean)
                .unwrap_or_default();
            let page = self.destination_page(item);
            self.out.push(Bookmark { level, title, page });
            self.items(item.get(b"First"), level + 1);
            node = item.get(b"Next");
        }
    }

    fn destination_page(&self, item: &'f Dict) -> Option<usize> {
        let file = self.file;
        let destination = match item.get(b"Dest") {
            Some(destination) => file.resolve(destination),
            None => {
                let action = file.dict(file.lookup(item, b"A"))?;
                if action.name(b"S") != Some(b"GoTo") {
                    return None;
                }
                file.lookup(action, b"D")
            }
        };
        self.page_of(destination, 0)
    }

    fn page_of(&self, destination: &'f Object, depth: usize) -> Option<usize> {
        if depth > 4 {
            return None;
        }
        match destination {
            Object::Array(items) => match items.first()? {
                Object::Ref(number, _) => self.numbers.get(number).copied(),
                Object::Int(index) => usize::try_from(*index).ok().map(|index| index + 1),
                _ => None,
            },
            Object::Name(name) | Object::String(name) => {
                self.page_of(self.names.get(name.as_slice())?, depth + 1)
            }
            Object::Dict(dict) => self.page_of(self.file.lookup(dict, b"D"), depth + 1),
            _ => None,
        }
    }
}

fn named_destinations<'f>(file: &'f File<'_>, catalog: &'f Dict) -> HashMap<Vec<u8>, &'f Object> {
    let mut names = HashMap::new();
    if let Some(dests) = file.dict(file.lookup(catalog, b"Dests")) {
        for (key, value) in dests.iter().take(MAX_NAMES) {
            names.insert(key.to_vec(), file.resolve(value));
        }
    }
    let Some(tree) = file
        .dict(file.lookup(catalog, b"Names"))
        .and_then(|names| file.dict(file.lookup(names, b"Dests")))
    else {
        return names;
    };
    let mut stack = vec![(tree, 0usize)];
    let mut seen = HashSet::new();
    while let Some((node, depth)) = stack.pop() {
        if let Some(pairs) = file.lookup(node, b"Names").as_array() {
            for [key, value] in pairs.as_chunks::<2>().0 {
                if names.len() >= MAX_NAMES {
                    return names;
                }
                if let Some(key) = file.resolve(key).as_string() {
                    names.insert(key.to_vec(), file.resolve(value));
                }
            }
        }
        if depth < MAX_OUTLINE_DEPTH
            && let Some(kids) = file.lookup(node, b"Kids").as_array()
        {
            for kid in kids {
                if let Some((number, _)) = kid.as_reference()
                    && !seen.insert(number)
                {
                    continue;
                }
                if let Some(kid) = file.dict(kid) {
                    stack.push((kid, depth + 1));
                }
            }
        }
    }
    names
}
