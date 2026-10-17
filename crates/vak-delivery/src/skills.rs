//! Card forms and presentation skills (docs/design/92-card-forms.md).
//!
//! A card is a **form**: a shape of information, drawn by one renderer on
//! every surface. The built-in forms are [`FORMS`]; each is the kind of the
//! card tool of the same name, and this module holds the one validator and
//! the one text form of each. A kind a pack adds is drawn as its block's
//! form; a kind a plugin skill provides is validated against the skill's
//! own schema and shown as text.
//!
//! Skills are **data**, not code: a plugin ships a
//! [`PresentationSkillManifest`] that is serialized into the delivery job and
//! sent to the isolated worker, which validates ```vak fences against it
//! without filesystem or network access.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub const PRESENTATION_SKILL_API: &str = "presentation.v1";

/// The built-in forms, in the order a person meets them in an answer.
pub const FORMS: &[&str] = &[
    "summary",
    "steps",
    "list",
    "details",
    "table",
    "comparison",
    "stats",
    "chart",
    "callout",
    "sources",
    "calendar",
    "map",
    "controls",
    "link",
    "media",
    "file_preview",
    "markup_preview",
    "diff",
    "test_report",
    "terminal",
];

/// Whether `kind` is a built-in form.
pub fn is_form(kind: &str) -> bool {
    FORMS.contains(&kind)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationSkillManifest {
    pub id: String,
    pub version: String,
    pub api: String,
    #[serde(default)]
    pub provides: Vec<String>,
    /// JSON Schema subset (`type`, `required`) the payload is checked
    /// against. A plugin kind is shown as text; its schema is what keeps
    /// that text from being whatever the model wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
}

/// A validated card: its kind, the form that draws it, and its payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredOutput {
    pub semantic_type: String,
    /// The form that draws it: the kind itself for a built-in form, the
    /// block a pack kind belongs to, empty for a plugin skill's kind.
    #[serde(default)]
    pub block: String,
    pub schema_version: u16,
    pub skill_id: String,
    pub skill_version: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillError {
    InvalidManifest(String),
    UnknownType(String),
    InvalidPayload(String),
}

impl std::fmt::Display for SkillError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidManifest(reason) => write!(f, "invalid manifest: {reason}"),
            Self::UnknownType(kind) => write!(f, "unknown semantic type: {kind}"),
            Self::InvalidPayload(reason) => write!(f, "invalid payload: {reason}"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRegistry {
    pub skills: BTreeMap<String, PresentationSkillManifest>,
}

const CORE_SKILL: &str = "core";
const CORE_VERSION: &str = "2.0.0";

impl SkillRegistry {
    pub fn register(&mut self, manifest: PresentationSkillManifest) -> Result<(), SkillError> {
        if manifest.id.trim().is_empty() || manifest.version.trim().is_empty() {
            return Err(SkillError::InvalidManifest(
                "id and version are required".into(),
            ));
        }
        if manifest.api != PRESENTATION_SKILL_API {
            return Err(SkillError::InvalidManifest(format!(
                "unsupported API {}",
                manifest.api
            )));
        }
        if manifest.provides.iter().any(|kind| kind.trim().is_empty()) {
            return Err(SkillError::InvalidManifest("empty semantic type".into()));
        }
        if manifest.id != CORE_SKILL && manifest.provides.iter().any(|kind| is_form(kind)) {
            return Err(SkillError::InvalidManifest(
                "a built-in form is not a plugin kind".into(),
            ));
        }
        if self.skills.iter().any(|(id, existing)| {
            id != &manifest.id
                && manifest
                    .provides
                    .iter()
                    .any(|kind| existing.provides.contains(kind))
        }) {
            return Err(SkillError::InvalidManifest(
                "semantic type is already owned by another skill".into(),
            ));
        }
        self.skills.insert(manifest.id.clone(), manifest);
        Ok(())
    }

    /// Merge the skill manifests plugins declare in `components.presentation`.
    /// A file that is not a manifest is another system's and is skipped.
    pub fn merge_plugin_files(&mut self, files: &[(String, Vec<u8>)]) {
        for (_path, bytes) in files {
            if let Ok(json) = std::str::from_utf8(bytes)
                && let Ok(manifest) = serde_json::from_str::<PresentationSkillManifest>(json)
            {
                let _ = self.register(manifest);
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<&PresentationSkillManifest> {
        self.skills.get(id)
    }

    /// The skill that provides `semantic_type`, as `(id, version)`.
    pub fn find_by_type(&self, semantic_type: &str) -> Option<(&String, &String)> {
        self.skills
            .values()
            .find(|skill| skill.provides.iter().any(|kind| kind == semantic_type))
            .map(|skill| (&skill.id, &skill.version))
    }

    /// Checks a card against the skill that owns its kind: a built-in form
    /// (or a pack kind, by its block) against the form's shape, a plugin
    /// kind against its skill's schema.
    pub fn validate(&self, output: &StructuredOutput) -> Result<(), SkillError> {
        let skill = self
            .get(&output.skill_id)
            .ok_or_else(|| SkillError::UnknownType(output.skill_id.clone()))?;
        if output.schema_version != crate::PRESENTATION_SCHEMA_VERSION
            || output.skill_version != skill.version
        {
            return Err(SkillError::InvalidPayload(
                "unsupported structured output version".into(),
            ));
        }
        if payload_size(&output.payload) > 1_000_000 {
            return Err(SkillError::InvalidPayload("payload exceeds 1 MB".into()));
        }
        if is_form(&output.block) {
            return validate_form(&output.block, &output.payload);
        }
        if !skill.provides.contains(&output.semantic_type) {
            return Err(SkillError::UnknownType(output.semantic_type.clone()));
        }
        match &skill.schema {
            Some(schema) => validate_against_schema(&output.payload, schema),
            None => Err(SkillError::InvalidPayload(format!(
                "skill {} declares no schema for {}",
                skill.id, output.semantic_type
            ))),
        }
    }
}

fn payload_size(payload: &Value) -> usize {
    serde_json::to_vec(payload).map_or(usize::MAX, |bytes| bytes.len())
}

/// The registry of the built-in forms alone.
pub fn built_in_skill_registry() -> SkillRegistry {
    let mut registry = SkillRegistry::default();
    let _ = registry.register(PresentationSkillManifest {
        id: CORE_SKILL.into(),
        version: CORE_VERSION.into(),
        api: PRESENTATION_SKILL_API.into(),
        provides: FORMS.iter().map(|form| (*form).to_string()).collect(),
        schema: None,
    });
    registry
}

/// Every kind the built-in registry accepts.
pub fn built_in_semantic_types() -> Vec<String> {
    FORMS.iter().map(|form| (*form).to_string()).collect()
}

/// A card for `kind` with `payload`, validated: a built-in form, or a kind
/// a plugin skill provides. A pack kind is resolved by the host, which knows
/// the packs, and arrives here as its block.
pub fn structured_output(
    kind: &str,
    block: Option<&str>,
    payload: Value,
    skills: &SkillRegistry,
) -> Result<StructuredOutput, SkillError> {
    let block = block.map(str::to_string).unwrap_or_else(|| {
        if is_form(kind) {
            kind.into()
        } else {
            String::new()
        }
    });
    let (skill_id, skill_version) = if is_form(kind) || is_form(&block) {
        (CORE_SKILL.to_string(), CORE_VERSION.to_string())
    } else {
        skills
            .find_by_type(kind)
            .map(|(id, version)| (id.clone(), version.clone()))
            .ok_or_else(|| SkillError::UnknownType(kind.into()))?
    };
    let output = StructuredOutput {
        semantic_type: kind.into(),
        block,
        schema_version: crate::PRESENTATION_SCHEMA_VERSION,
        skill_id,
        skill_version,
        payload,
    };
    skills.validate(&output)?;
    Ok(output)
}

/// A ```vak fence's body: `{"semantic_type": …, "payload": …}`.
pub fn parse_fragment(source: &str) -> Result<StructuredOutput, SkillError> {
    parse_fragment_with(source, &built_in_skill_registry())
}

/// [`parse_fragment`] against a plugin-extended registry.
pub fn parse_fragment_with(
    source: &str,
    skills: &SkillRegistry,
) -> Result<StructuredOutput, SkillError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Fragment {
        semantic_type: String,
        payload: Value,
    }
    let fragment: Fragment = serde_json::from_str(source.trim())
        .map_err(|err| SkillError::InvalidPayload(err.to_string()))?;
    structured_output(&fragment.semantic_type, None, fragment.payload, skills)
}

/// Every valid ```vak fence in `text`, in order, each once.
pub fn structured_outputs_from_text(text: &str) -> Vec<StructuredOutput> {
    structured_outputs_from_text_with(text, &built_in_skill_registry())
}

/// [`structured_outputs_from_text`] against a plugin-extended registry.
pub fn structured_outputs_from_text_with(
    text: &str,
    skills: &SkillRegistry,
) -> Vec<StructuredOutput> {
    let mut outputs: Vec<StructuredOutput> = Vec::new();
    for body in vak_fences(text) {
        if let Ok(output) = parse_fragment_with(&body, skills)
            && !outputs.contains(&output)
        {
            outputs.push(output);
        }
    }
    outputs
}

/// The bodies of every ```vak fence in `source`, as Markdown parses them.
fn vak_fences(source: &str) -> Vec<String> {
    use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
    let mut bodies = Vec::new();
    let mut pending: Option<String> = None;
    for event in Parser::new(source) {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(language)))
                if language.as_ref() == "vak" =>
            {
                pending = Some(String::new());
            }
            Event::Text(text) => {
                if let Some(body) = &mut pending {
                    body.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(body) = pending.take() {
                    bodies.push(body);
                }
            }
            _ => {}
        }
    }
    bodies
}

/// Replace each valid ```vak fence with its card's text form. Ordinary
/// Markdown and invalid fences stay exactly as written.
pub fn project_structured_fences(source: &str) -> String {
    project_structured_fences_with(source, &built_in_skill_registry())
}

/// [`project_structured_fences`] against a plugin-extended registry.
pub fn project_structured_fences_with(source: &str, skills: &SkillRegistry) -> String {
    use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
    let mut replacements = Vec::new();
    let mut pending: Option<(usize, String)> = None;
    for (event, range) in Parser::new(source).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(language)))
                if language.as_ref() == "vak" =>
            {
                pending = Some((range.start, String::new()));
            }
            Event::Text(text) => {
                if let Some((_, content)) = &mut pending {
                    content.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((start, content)) = pending.take()
                    && let Ok(output) = parse_fragment_with(&content, skills)
                {
                    replacements.push((start..range.end, structured_markdown(&output)));
                }
            }
            _ => {}
        }
    }
    let mut result = source.to_string();
    for (range, replacement) in replacements.into_iter().rev() {
        result.replace_range(range, &replacement);
    }
    result
}

// ---- validation ------------------------------------------------------------

type Check = Result<(), SkillError>;

fn invalid(form: &str, reason: impl std::fmt::Display) -> SkillError {
    SkillError::InvalidPayload(format!("{form}: {reason}"))
}

/// One record of a form: its fields, with what each must be.
struct Record<'a> {
    form: &'a str,
    at: String,
    fields: &'a Map<String, Value>,
}

impl<'a> Record<'a> {
    fn of(form: &'a str, at: impl Into<String>, value: &'a Value) -> Result<Self, SkillError> {
        let at = at.into();
        let fields = value
            .as_object()
            .ok_or_else(|| invalid(form, format!("{at} must be an object")))?;
        Ok(Self { form, at, fields })
    }

    fn err(&self, reason: impl std::fmt::Display) -> SkillError {
        if self.at.is_empty() {
            invalid(self.form, reason)
        } else {
            invalid(self.form, format!("{}: {reason}", self.at))
        }
    }

    /// Only these keys (and `label`, the eyebrow every card may carry at
    /// its top) appear.
    fn only(&self, keys: &[&str]) -> Check {
        let top = self.at.is_empty();
        match self
            .fields
            .keys()
            .find(|key| !keys.contains(&key.as_str()) && !(top && key.as_str() == "label"))
        {
            Some(key) => Err(self.err(format!("unexpected field `{key}`"))),
            None => Ok(()),
        }
    }

    fn text(&self, key: &str) -> Result<&'a str, SkillError> {
        self.fields
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| self.err(format!("`{key}` must be non-empty text")))
    }

    fn optional_text(&self, key: &str) -> Check {
        match self.fields.get(key) {
            None | Some(Value::String(_)) => Ok(()),
            Some(_) => Err(self.err(format!("`{key}` must be text"))),
        }
    }

    fn texts(&self, keys: &[&str]) -> Check {
        keys.iter().try_for_each(|key| self.optional_text(key))
    }

    fn list(&self, key: &str) -> Result<&'a [Value], SkillError> {
        self.fields
            .get(key)
            .and_then(Value::as_array)
            .filter(|items| !items.is_empty() && items.len() <= 10_000)
            .map(Vec::as_slice)
            .ok_or_else(|| self.err(format!("`{key}` must be a non-empty list")))
    }

    fn optional_list(&self, key: &str) -> Result<&'a [Value], SkillError> {
        match self.fields.get(key) {
            None => Ok(&[]),
            Some(_) => self.list(key),
        }
    }

    fn whole(&self, key: &str) -> Check {
        match self.fields.get(key) {
            None => Ok(()),
            Some(value) if value.is_u64() => Ok(()),
            Some(_) => Err(self.err(format!("`{key}` must be a whole number, 0 or more"))),
        }
    }

    fn one_of(&self, key: &str, allowed: &[&str]) -> Check {
        let value = self.text(key)?;
        if allowed.contains(&value) {
            Ok(())
        } else {
            Err(self.err(format!("`{key}` must be one of {allowed:?}")))
        }
    }

    fn records(&self, key: &str, optional: bool) -> Result<Vec<Record<'a>>, SkillError> {
        let items = if optional {
            self.optional_list(key)?
        } else {
            self.list(key)?
        };
        items
            .iter()
            .enumerate()
            .map(|(index, item)| Record::of(self.form, format!("{key}[{index}]"), item))
            .collect()
    }
}

fn scalar(value: &Value) -> bool {
    value.is_string() || value.is_number() || value.is_boolean()
}

fn strings(record: &Record<'_>, key: &str) -> Check {
    for (index, item) in record.optional_list(key)?.iter().enumerate() {
        if !item.as_str().is_some_and(|text| !text.trim().is_empty()) {
            return Err(record.err(format!("`{key}[{index}]` must be non-empty text")));
        }
    }
    Ok(())
}

fn date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && text
            .split('-')
            .all(|part| part.bytes().all(|b| b.is_ascii_digit()))
        && matches!(text[5..7].parse::<u8>(), Ok(1..=12))
        && matches!(text[8..10].parse::<u8>(), Ok(1..=31))
}

fn clock(text: &str) -> bool {
    let Some((hours, minutes)) = text.split_once(':') else {
        return false;
    };
    hours.len() == 2
        && minutes.len() == 2
        && matches!(hours.parse::<u8>(), Ok(0..=23))
        && matches!(minutes.parse::<u8>(), Ok(0..=59))
}

/// Checks a payload against the shape of `form`.
pub fn validate_form(form: &str, payload: &Value) -> Check {
    let card = Record::of(form, "", payload)?;
    match form {
        "summary" => {
            card.only(&["headline", "text", "points"])?;
            card.text("headline")?;
            card.optional_text("text")?;
            strings(&card, "points")
        }
        "steps" => {
            card.only(&["title", "steps"])?;
            card.optional_text("title")?;
            for step in card.records("steps", false)? {
                step.only(&["label", "detail", "time", "choices"])?;
                step.text("label")?;
                step.texts(&["detail", "time"])?;
                for option in step.records("choices", true)? {
                    option.only(&["label", "detail", "facts"])?;
                    option.text("label")?;
                    option.optional_text("detail")?;
                    strings(&option, "facts")?;
                }
            }
            Ok(())
        }
        "list" => {
            card.only(&["title", "checkable", "items"])?;
            card.optional_text("title")?;
            if card
                .fields
                .get("checkable")
                .is_some_and(|v| !v.is_boolean())
            {
                return Err(card.err("`checkable` must be true or false"));
            }
            for item in card.records("items", false)? {
                item.only(&["label", "detail", "url"])?;
                item.text("label")?;
                item.texts(&["detail", "url"])?;
            }
            Ok(())
        }
        "details" => {
            card.only(&["title", "text", "fields"])?;
            card.texts(&["title", "text"])?;
            for field in card.records("fields", false)? {
                field.only(&["label", "detail"])?;
                field.text("label")?;
                field.text("detail")?;
            }
            Ok(())
        }
        "table" => {
            card.only(&["title", "columns", "rows"])?;
            card.optional_text("title")?;
            let columns = card.records("columns", false)?;
            if columns.len() < 2 {
                return Err(card.err("a table has two or more columns"));
            }
            for column in &columns {
                column.only(&["label", "numeric"])?;
                column.text("label")?;
                if !column.fields.get("numeric").is_some_and(Value::is_boolean) {
                    return Err(column.err("`numeric` must be true or false"));
                }
            }
            for (index, row) in card.list("rows")?.iter().enumerate() {
                let cells = row
                    .as_array()
                    .filter(|cells| cells.len() == columns.len())
                    .ok_or_else(|| {
                        card.err(format!("rows[{index}] must have one cell per column"))
                    })?;
                if !cells.iter().all(scalar) {
                    return Err(card.err(format!("rows[{index}] cells must be text or numbers")));
                }
            }
            Ok(())
        }
        "comparison" => {
            card.only(&["title", "options", "aspects", "recommendation"])?;
            card.texts(&["title", "recommendation"])?;
            let options = card.records("options", false)?;
            if options.len() < 2 {
                return Err(card.err("a comparison has two or more options"));
            }
            for option in &options {
                option.only(&["name", "verdict", "strengths", "weaknesses"])?;
                option.text("name")?;
                option.optional_text("verdict")?;
                strings(option, "strengths")?;
                strings(option, "weaknesses")?;
            }
            for aspect in card.records("aspects", true)? {
                aspect.only(&["label", "values"])?;
                aspect.text("label")?;
                let values = aspect
                    .fields
                    .get("values")
                    .and_then(Value::as_array)
                    .filter(|values| {
                        values.len() == options.len() && values.iter().all(Value::is_string)
                    });
                if values.is_none() {
                    return Err(aspect.err("`values` holds one text per option"));
                }
            }
            Ok(())
        }
        "stats" => {
            card.only(&["title", "readings"])?;
            card.optional_text("title")?;
            for reading in card.records("readings", false)? {
                reading.only(&["label", "value", "unit", "change"])?;
                reading.text("label")?;
                if !reading.fields.get("value").is_some_and(scalar) {
                    return Err(reading.err("`value` must be text or a number"));
                }
                reading.texts(&["unit", "change"])?;
            }
            Ok(())
        }
        "chart" => {
            card.only(&[
                "title",
                "chart_type",
                "x_label",
                "y_label",
                "accessible_summary",
                "series",
            ])?;
            card.texts(&["title", "x_label", "y_label"])?;
            card.one_of("chart_type", &["line", "bar", "area"])?;
            card.text("accessible_summary")?;
            for series in card.records("series", false)? {
                series.only(&["name", "points"])?;
                series.optional_text("name")?;
                for point in series.records("points", false)? {
                    point.only(&["x", "y"])?;
                    if !point
                        .fields
                        .get("x")
                        .is_some_and(|x| x.is_string() || x.is_number())
                    {
                        return Err(point.err("`x` must be text or a number"));
                    }
                    if !point
                        .fields
                        .get("y")
                        .is_some_and(|y| y.is_null() || y.as_f64().is_some_and(f64::is_finite))
                    {
                        return Err(point.err("`y` must be a number"));
                    }
                }
            }
            Ok(())
        }
        "callout" => {
            card.only(&["tone", "title", "text"])?;
            card.one_of("tone", &["tip", "note", "warning", "important"])?;
            card.optional_text("title")?;
            card.text("text").map(drop)
        }
        "sources" => {
            card.only(&["title", "sources"])?;
            card.optional_text("title")?;
            for source in card.records("sources", false)? {
                source.only(&["title", "url", "finding", "source_name", "published_at"])?;
                source.text("title")?;
                source.text("url")?;
                source.text("finding")?;
                source.texts(&["source_name", "published_at"])?;
            }
            Ok(())
        }
        "calendar" => {
            card.only(&["title", "events"])?;
            card.optional_text("title")?;
            for event in card.records("events", false)? {
                event.only(&["title", "date", "start", "end", "location", "detail"])?;
                event.text("title")?;
                if !date(event.text("date")?) {
                    return Err(event.err("`date` must be YYYY-MM-DD"));
                }
                for key in ["start", "end"] {
                    if let Some(time) = event.fields.get(key)
                        && !time.as_str().is_some_and(clock)
                    {
                        return Err(event.err(format!("`{key}` must be HH:MM")));
                    }
                }
                event.texts(&["location", "detail"])?;
            }
            Ok(())
        }
        "map" => {
            card.only(&["title", "connect", "places"])?;
            card.optional_text("title")?;
            card.one_of("connect", &["none", "route", "area"])?;
            let places = card.records("places", false)?;
            for place in &places {
                place.only(&["label", "latitude", "longitude", "detail"])?;
                place.text("label")?;
                place.optional_text("detail")?;
                for (key, limit) in [("latitude", 90.0), ("longitude", 180.0)] {
                    if !place
                        .fields
                        .get(key)
                        .and_then(Value::as_f64)
                        .is_some_and(|degrees| degrees.is_finite() && degrees.abs() <= limit)
                    {
                        return Err(place.err(format!(
                            "`{key}` must be a number of degrees from -{limit} to {limit}"
                        )));
                    }
                }
            }
            match (text_of(payload, "connect"), places.len()) {
                ("route", 0..2) => Err(card.err("a route needs at least two places")),
                ("area", 0..3) => Err(card.err("an area needs at least three places")),
                _ => Ok(()),
            }
        }
        "controls" => validate_controls(&card, payload),
        "link" => {
            card.only(&["url", "title", "description", "site_name", "image_url"])?;
            card.text("url")?;
            card.text("title")?;
            card.texts(&["description", "site_name", "image_url"])
        }
        "media" => {
            card.only(&["format", "url", "alt", "title"])?;
            card.one_of("format", &["image", "video", "audio"])?;
            card.text("url")?;
            card.text("alt")?;
            card.optional_text("title")
        }
        "file_preview" => {
            card.only(&["artifact_path", "title", "status"])?;
            card.text("artifact_path")?;
            card.texts(&["title", "status"])
        }
        "markup_preview" => {
            card.only(&["html", "title", "status"])?;
            card.text("html")?;
            card.texts(&["title", "status"])
        }
        "diff" => {
            card.only(&["files"])?;
            for file in card.records("files", false)? {
                file.only(&["filename", "hunks", "additions", "deletions"])?;
                file.text("filename")?;
                file.text("hunks")?;
                for key in ["additions", "deletions"] {
                    if !file.fields.get(key).is_some_and(Value::is_u64) {
                        return Err(file.err(format!("`{key}` must be a whole number")));
                    }
                }
            }
            Ok(())
        }
        "test_report" => {
            card.only(&["command", "suite_name", "tests", "exit_code", "duration_ms"])?;
            card.text("command")?;
            card.optional_text("suite_name")?;
            card.whole("duration_ms")?;
            if card
                .fields
                .get("exit_code")
                .is_some_and(|code| !code.is_i64())
            {
                return Err(card.err("`exit_code` must be a whole number"));
            }
            for test in card.records("tests", false)? {
                test.only(&["name", "status", "duration_ms", "message"])?;
                test.text("name")?;
                test.one_of("status", &["passed", "failed", "skipped"])?;
                test.whole("duration_ms")?;
                test.optional_text("message")?;
            }
            Ok(())
        }
        "terminal" => {
            card.only(&["command", "output", "exit_code", "duration_ms"])?;
            card.text("command")?;
            if !card.fields.get("output").is_some_and(Value::is_string) {
                return Err(card.err("`output` must be text"));
            }
            card.whole("duration_ms")?;
            if card
                .fields
                .get("exit_code")
                .is_some_and(|code| !code.is_i64())
            {
                return Err(card.err("`exit_code` must be a whole number"));
            }
            Ok(())
        }
        other => Err(SkillError::UnknownType(other.into())),
    }
}

/// A plugin schema's subset: `type` and `required`.
fn validate_against_schema(payload: &Value, schema: &Value) -> Check {
    let Some(schema) = schema.as_object() else {
        return Err(SkillError::InvalidPayload(
            "plugin schema must be a JSON object".into(),
        ));
    };
    if let Some(expected) = schema.get("type").and_then(Value::as_str) {
        let matches = match expected {
            "object" => payload.is_object(),
            "array" => payload.is_array(),
            "string" => payload.is_string(),
            "number" => payload.is_number(),
            "boolean" => payload.is_boolean(),
            "null" => payload.is_null(),
            _ => true,
        };
        if !matches {
            return Err(SkillError::InvalidPayload(format!(
                "payload is not a {expected}"
            )));
        }
    }
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        let object = payload.as_object().ok_or_else(|| {
            SkillError::InvalidPayload("payload must be an object for required check".into())
        })?;
        if let Some(key) = required
            .iter()
            .filter_map(Value::as_str)
            .find(|key| !object.contains_key(*key))
        {
            return Err(SkillError::InvalidPayload(format!(
                "missing required field: {key}"
            )));
        }
    }
    Ok(())
}

/// A controls card's input or output name: an identifier the expressions
/// can read, used once.
fn control_name(record: &Record<'_>, names: &mut Vec<String>) -> Check {
    let name = record.text("name")?;
    let readable = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !readable || crate::expression::function_names().contains(&name) {
        return Err(record.err(format!(
            "`name` `{name}` must be letters, digits and _, start with a letter, and not be a function's name"
        )));
    }
    if names.iter().any(|known| known == name) {
        return Err(record.err(format!("`name` `{name}` is used twice")));
    }
    names.push(name.to_string());
    Ok(())
}

/// A controls card (docs/design/96, P1): typed inputs, outputs computed
/// from them in the closed expression language, and the card's own values
/// as a worked example every output must agree with.
fn validate_controls(card: &Record<'_>, payload: &Value) -> Check {
    card.only(&["title", "inputs", "outputs"])?;
    card.optional_text("title")?;
    let mut names: Vec<String> = Vec::new();
    for input in card.records("inputs", false)? {
        input.only(&[
            "name", "label", "kind", "value", "unit", "min", "max", "step", "options",
        ])?;
        control_name(&input, &mut names)?;
        input.text("label")?;
        input.optional_text("unit")?;
        input.one_of("kind", crate::controls::INPUT_KINDS)?;
        let bound = |key: &str| -> Result<Option<f64>, SkillError> {
            match input.fields.get(key) {
                None => Ok(None),
                Some(value) => value
                    .as_f64()
                    .filter(|number| number.is_finite())
                    .map(Some)
                    .ok_or_else(|| input.err(format!("`{key}` must be a number"))),
            }
        };
        let (min, max, step) = (bound("min")?, bound("max")?, bound("step")?);
        if step.is_some_and(|step| step <= 0.0) {
            return Err(input.err("`step` must be greater than zero"));
        }
        let kind = text_of(&Value::Object(input.fields.clone()), "kind").to_string();
        match kind.as_str() {
            "toggle" if !input.fields.get("value").is_some_and(Value::is_boolean) => {
                return Err(input.err("a toggle's `value` must be true or false"));
            }
            "date"
                if !input
                    .fields
                    .get("value")
                    .and_then(Value::as_str)
                    .is_some_and(date) =>
            {
                return Err(input.err("a date's `value` must be YYYY-MM-DD"));
            }
            "number" | "range" | "choice" => {
                let Some(value) = input.fields.get("value").and_then(Value::as_f64) else {
                    return Err(input.err(format!("a {kind}'s `value` must be a number")));
                };
                if kind == "range" && (min.is_none() || max.is_none()) {
                    return Err(input.err("a range needs `min` and `max`"));
                }
                if min.is_some_and(|min| value < min) || max.is_some_and(|max| value > max) {
                    return Err(input.err("`value` must lie between `min` and `max`"));
                }
                if kind == "choice" {
                    let options = input.records("options", false)?;
                    for option in &options {
                        option.only(&["label", "value"])?;
                        option.text("label")?;
                        if !option
                            .fields
                            .get("value")
                            .and_then(Value::as_f64)
                            .is_some_and(f64::is_finite)
                        {
                            return Err(option.err("`value` must be a number"));
                        }
                    }
                    if !options.iter().any(|option| {
                        option.fields.get("value").and_then(Value::as_f64) == Some(value)
                    }) {
                        return Err(
                            input.err("a choice's `value` must be one of its options' values")
                        );
                    }
                }
            }
            _ => {}
        }
    }
    for output in card.records("outputs", false)? {
        output.only(&["name", "label", "expression", "unit", "equals", "decimals"])?;
        control_name(&output, &mut names)?;
        output.text("label")?;
        output.optional_text("unit")?;
        output.whole("decimals")?;
        let source = output.text("expression")?;
        let expr = crate::expression::parse(source)
            .map_err(|error| output.err(format!("`expression`: {error}")))?;
        let known = &names[..names.len() - 1];
        if let Some(unknown) = crate::expression::names(&expr)
            .into_iter()
            .find(|read| !known.contains(read))
        {
            return Err(output.err(format!(
                "`expression` reads `{unknown}`, which is not an input or an earlier output (those are {})",
                known.join(", ")
            )));
        }
        if !output
            .fields
            .get("equals")
            .and_then(Value::as_f64)
            .is_some_and(f64::is_finite)
        {
            return Err(output.err("`equals` must be a number"));
        }
    }
    let computed = crate::controls::compute(payload, &std::collections::BTreeMap::new())
        .map_err(|error| card.err(error))?;
    for (output, (name, value)) in items(payload, "outputs").iter().zip(computed) {
        let stated = output["equals"].as_f64().unwrap_or_default();
        let decimals = output["decimals"].as_u64().unwrap_or(0);
        if !crate::controls::agrees(value, stated, decimals) {
            return Err(card.err(format!(
                "output `{name}` computes {} from the inputs' values, not the {} given as `equals`",
                crate::controls::shown(value, decimals),
                crate::controls::shown(stated, decimals)
            )));
        }
    }
    Ok(())
}

// ---- text form -------------------------------------------------------------

fn text_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
}

fn items<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn shown(value: &Value) -> String {
    match value {
        Value::String(text) => text.trim().to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn joined<'a>(parts: impl IntoIterator<Item = &'a str>, separator: &str) -> String {
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(separator)
}

fn cell(value: &Value) -> String {
    shown(value).replace('|', "\\|").replace('\n', " ")
}

/// The card as Markdown a person reads: its own text form, by its form, on
/// every surface that draws no card (channels, exports, the model's view of
/// an earlier turn).
pub fn structured_markdown(output: &StructuredOutput) -> String {
    let p = &output.payload;
    let mut lines: Vec<String> = Vec::new();
    let heading = joined([text_of(p, "label"), text_of(p, "title")], " · ");
    if !heading.is_empty() && output.block != "summary" {
        lines.push(format!("**{heading}**"));
    }
    match output.block.as_str() {
        "summary" => {
            lines.push(format!("**{}**", text_of(p, "headline")));
            if !text_of(p, "text").is_empty() {
                lines.push(text_of(p, "text").into());
            }
            for point in items(p, "points") {
                lines.push(format!("- {}", shown(point)));
            }
        }
        "steps" => {
            for (index, step) in items(p, "steps").iter().enumerate() {
                let head = joined([text_of(step, "time"), text_of(step, "label")], " · ");
                let detail = text_of(step, "detail");
                lines.push(if detail.is_empty() {
                    format!("{}. {head}", index + 1)
                } else {
                    format!("{}. {head}: {detail}", index + 1)
                });
                for option in items(step, "choices") {
                    let facts: Vec<String> = items(option, "facts").iter().map(shown).collect();
                    let about = joined(
                        [text_of(option, "detail"), facts.join(", ").as_str()],
                        " · ",
                    );
                    lines.push(if about.is_empty() {
                        format!("   - Option: {}", text_of(option, "label"))
                    } else {
                        format!("   - Option: {}: {about}", text_of(option, "label"))
                    });
                }
            }
        }
        "list" => {
            let mark = if p.get("checkable").and_then(Value::as_bool) == Some(true) {
                "- [ ] "
            } else {
                "- "
            };
            for item in items(p, "items") {
                let label = match text_of(item, "url") {
                    "" => text_of(item, "label").to_string(),
                    url => format!("[{}]({url})", text_of(item, "label")),
                };
                let detail = text_of(item, "detail");
                lines.push(if detail.is_empty() {
                    format!("{mark}{label}")
                } else {
                    format!("{mark}{label}: {detail}")
                });
            }
        }
        "details" => {
            if !text_of(p, "text").is_empty() {
                lines.push(text_of(p, "text").into());
            }
            for field in items(p, "fields") {
                lines.push(format!(
                    "- **{}**: {}",
                    text_of(field, "label"),
                    text_of(field, "detail")
                ));
            }
        }
        "table" => {
            let labels: Vec<String> = items(p, "columns")
                .iter()
                .map(|column| cell(&column["label"]))
                .collect();
            lines.push(format!("| {} |", labels.join(" | ")));
            lines.push(format!("|{}|", vec![" --- "; labels.len()].join("|")));
            for row in items(p, "rows") {
                let cells: Vec<String> = row.as_array().into_iter().flatten().map(cell).collect();
                lines.push(format!("| {} |", cells.join(" | ")));
            }
        }
        "comparison" => {
            for option in items(p, "options") {
                let verdict = text_of(option, "verdict");
                lines.push(if verdict.is_empty() {
                    format!("- **{}**", text_of(option, "name"))
                } else {
                    format!("- **{}**: {verdict}", text_of(option, "name"))
                });
                for (key, word) in [("strengths", "+"), ("weaknesses", "−")] {
                    for point in items(option, key) {
                        lines.push(format!("  {word} {}", shown(point)));
                    }
                }
            }
            let names: Vec<String> = items(p, "options")
                .iter()
                .map(|option| cell(&option["name"]))
                .collect();
            if !items(p, "aspects").is_empty() {
                lines.push(format!("| | {} |", names.join(" | ")));
                lines.push(format!("|---|{}|", vec![" --- "; names.len()].join("|")));
                for aspect in items(p, "aspects") {
                    let values: Vec<String> = items(aspect, "values").iter().map(cell).collect();
                    lines.push(format!(
                        "| {} | {} |",
                        cell(&aspect["label"]),
                        values.join(" | ")
                    ));
                }
            }
            if !text_of(p, "recommendation").is_empty() {
                lines.push(format!("Recommendation: {}", text_of(p, "recommendation")));
            }
        }
        "stats" => {
            for reading in items(p, "readings") {
                let value = joined(
                    [shown(&reading["value"]).as_str(), text_of(reading, "unit")],
                    " ",
                );
                let change = text_of(reading, "change");
                lines.push(if change.is_empty() {
                    format!("- {}: {value}", text_of(reading, "label"))
                } else {
                    format!("- {}: {value} ({change})", text_of(reading, "label"))
                });
            }
        }
        "chart" => {
            lines.push(text_of(p, "accessible_summary").into());
            for series in items(p, "series") {
                let points: Vec<String> = items(series, "points")
                    .iter()
                    .map(|point| format!("{} {}", shown(&point["x"]), shown(&point["y"])))
                    .collect();
                let name = text_of(series, "name");
                lines.push(format!(
                    "- {}{}",
                    if name.is_empty() {
                        String::new()
                    } else {
                        format!("{name}: ")
                    },
                    points.join(", ")
                ));
            }
        }
        "callout" => lines.push(format!("> {}", text_of(p, "text"))),
        "sources" => {
            for (index, source) in items(p, "sources").iter().enumerate() {
                let by = joined(
                    [
                        text_of(source, "source_name"),
                        text_of(source, "published_at"),
                    ],
                    ", ",
                );
                let by = if by.is_empty() {
                    by
                } else {
                    format!(" ({by})")
                };
                lines.push(format!(
                    "{}. {} [{}]({}){by}",
                    index + 1,
                    text_of(source, "finding"),
                    text_of(source, "title"),
                    text_of(source, "url"),
                ));
            }
        }
        "calendar" => {
            let mut day = "";
            for event in items(p, "events") {
                if text_of(event, "date") != day {
                    day = text_of(event, "date");
                    lines.push(format!("**{day}**"));
                }
                let time = joined([text_of(event, "start"), text_of(event, "end")], "–");
                let about = joined(
                    [text_of(event, "location"), text_of(event, "detail")],
                    " · ",
                );
                let head = joined([time.as_str(), text_of(event, "title")], " ");
                lines.push(if about.is_empty() {
                    format!("- {head}")
                } else {
                    format!("- {head}: {about}")
                });
            }
        }
        "map" => {
            match text_of(p, "connect") {
                "route" => lines.push("A route through these places, in order:".into()),
                "area" => lines.push("An area with these corners, in order:".into()),
                _ => {}
            }
            for (index, place) in items(p, "places").iter().enumerate() {
                let detail = text_of(place, "detail");
                lines.push(format!(
                    "{}. {} ({}, {}){}",
                    index + 1,
                    text_of(place, "label"),
                    shown(&place["latitude"]),
                    shown(&place["longitude"]),
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(": {detail}")
                    }
                ));
            }
        }
        "controls" => {
            let with_unit = |value: String, item: &Value| match text_of(item, "unit") {
                "" => value,
                unit => format!("{value} {unit}"),
            };
            lines.push("| | |".into());
            lines.push("| --- | --- |".into());
            for input in items(p, "inputs") {
                let value = match &input["value"] {
                    Value::Bool(on) => if *on { "on" } else { "off" }.to_string(),
                    other => shown(other),
                };
                lines.push(format!(
                    "| {} | {} |",
                    text_of(input, "label"),
                    with_unit(value, input)
                ));
            }
            for output in items(p, "outputs") {
                let decimals = output["decimals"].as_u64().unwrap_or(0);
                let value =
                    crate::controls::shown(output["equals"].as_f64().unwrap_or_default(), decimals);
                lines.push(format!(
                    "| **{}** | **{}** |",
                    text_of(output, "label"),
                    with_unit(value, output)
                ));
            }
        }
        "link" => {
            lines.push(format!("[{}]({})", text_of(p, "title"), text_of(p, "url")));
            if !text_of(p, "description").is_empty() {
                lines.push(text_of(p, "description").into());
            }
        }
        "media" => lines.push(format!(
            "{}: [{}]({})",
            text_of(p, "format"),
            text_of(p, "alt"),
            text_of(p, "url")
        )),
        "file_preview" => lines.push(format!("Preview of `{}`", text_of(p, "artifact_path"))),
        "markup_preview" => lines.push("A preview of HTML markup.".into()),
        "diff" => {
            for file in items(p, "files") {
                lines.push(format!(
                    "- `{}` +{} −{}",
                    text_of(file, "filename"),
                    shown(&file["additions"]),
                    shown(&file["deletions"])
                ));
            }
        }
        "test_report" => {
            let count = |status: &str| {
                items(p, "tests")
                    .iter()
                    .filter(|test| text_of(test, "status") == status)
                    .count()
            };
            lines.push(format!(
                "`{}`: {} passed, {} failed, {} skipped",
                text_of(p, "command"),
                count("passed"),
                count("failed"),
                count("skipped")
            ));
            for test in items(p, "tests")
                .iter()
                .filter(|test| text_of(test, "status") == "failed")
            {
                lines.push(format!(
                    "- {}: {}",
                    text_of(test, "name"),
                    text_of(test, "message")
                ));
            }
        }
        "terminal" => {
            let output = text_of(p, "output");
            let fence = "`".repeat(longest_backtick_run(output).max(2) + 1);
            lines.push(format!(
                "{fence}\n$ {}\n{output}\n{fence}",
                text_of(p, "command")
            ));
        }
        _ => append_readable_fields(p, "", &mut lines),
    }
    lines.join("\n\n")
}

fn longest_backtick_run(text: &str) -> usize {
    text.split(|c| c != '`').map(str::len).max().unwrap_or(0)
}

/// A plugin kind's fields, each on its own line by its path.
fn append_readable_fields(value: &Value, path: &str, lines: &mut Vec<String>) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if path.is_empty() && matches!(key.as_str(), "title" | "label") {
                    continue;
                }
                let child = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path} › {key}")
                };
                append_readable_fields(value, &child, lines);
            }
        }
        Value::Array(items) => {
            for (index, value) in items.iter().enumerate() {
                append_readable_fields(value, &format!("{path} · {}", index + 1), lines);
            }
        }
        Value::Null => {}
        Value::String(text) if text.is_empty() => {}
        other => lines.push(format!("{path}: {}", shown(other))),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;

    /// One valid payload per form: the shape the card tools build.
    pub(crate) fn sample(form: &str) -> Value {
        match form {
            "summary" => json!({"headline": "Yes", "text": "Because.", "points": ["One"]}),
            "steps" => json!({"steps": [{"label": "Boil", "time": "9:00",
                "choices": [{"label": "Kettle", "facts": ["fast"]}]}]}),
            "list" => {
                json!({"checkable": true, "items": [{"label": "Rice", "url": "https://a.example"}]})
            }
            "details" => json!({"title": "Pune", "fields": [{"label": "State", "detail": "MH"}]}),
            "table" => {
                json!({"columns": [{"label": "Item", "numeric": false}, {"label": "Kg", "numeric": true}],
                "rows": [["Rice", 2]]})
            }
            "comparison" => json!({"options": [{"name": "A", "strengths": ["x"]}, {"name": "B"}],
                "aspects": [{"label": "Cost", "values": ["Low", "High"]}], "recommendation": "A"}),
            "stats" => json!({"readings": [{"label": "Uptime", "value": 99.9, "unit": "%"}]}),
            "chart" => json!({"chart_type": "bar", "accessible_summary": "Up.",
                "series": [{"name": "s", "points": [{"x": "Q1", "y": 2}]}]}),
            "callout" => json!({"tone": "warning", "text": "Mind the gap."}),
            "sources" => {
                json!({"sources": [{"title": "T", "url": "https://a.example", "finding": "F"}]})
            }
            "calendar" => {
                json!({"events": [{"title": "Standup", "date": "2026-10-15", "start": "09:00"}]})
            }
            "map" => json!({"connect": "route", "places": [
                {"label": "Station", "latitude": 18.5286, "longitude": 73.8746},
                {"label": "Fort", "latitude": 18.5195, "longitude": 73.8553, "detail": "Gates at 9"}]}),
            "controls" => json!({"inputs": [
                {"name": "bill", "label": "Bill", "kind": "number", "value": 1200, "unit": "₹"},
                {"name": "people", "label": "People", "kind": "range", "value": 4, "min": 1, "max": 12, "step": 1}],
                "outputs": [{"name": "each", "label": "Each pays", "expression": "bill / people",
                "equals": 300, "decimals": 0, "unit": "₹"}]}),
            "link" => json!({"url": "https://a.example", "title": "A"}),
            "media" => json!({"format": "image", "url": "https://a.example/a.png", "alt": "A"}),
            "file_preview" => json!({"artifact_path": "index.html"}),
            "markup_preview" => json!({"html": "<p>Hi</p>"}),
            "diff" => {
                json!({"files": [{"filename": "a.rs", "hunks": "@@", "additions": 1, "deletions": 0}]})
            }
            "test_report" => json!({"command": "cargo test", "duration_ms": 5, "exit_code": 0,
                "tests": [{"name": "t", "status": "passed"}]}),
            "terminal" => json!({"command": "ls", "output": "a.rs", "exit_code": 0}),
            other => panic!("no sample for {other}"),
        }
    }

    #[test]
    fn every_form_takes_its_own_shape_and_refuses_a_stray_field() {
        let skills = built_in_skill_registry();
        for form in FORMS {
            let output = structured_output(form, None, sample(form), &skills)
                .unwrap_or_else(|e| panic!("{form}: {e}"));
            assert_eq!(output.block, *form);
            let mut stray = sample(form);
            stray["stray"] = json!(1);
            assert!(
                structured_output(form, None, stray, &skills).is_err(),
                "{form}"
            );
            let mut labelled = sample(form);
            labelled["label"] = json!("Eyebrow");
            assert!(
                structured_output(form, None, labelled, &skills).is_ok(),
                "{form}"
            );
        }
    }

    #[test]
    fn every_form_has_a_text_form_that_carries_its_content() {
        let skills = built_in_skill_registry();
        for form in FORMS {
            let output = structured_output(form, None, sample(form), &skills).unwrap();
            let text = structured_markdown(&output);
            assert!(!text.trim().is_empty(), "{form}");
            assert!(!text.contains("{\""), "{form} leaked JSON: {text}");
        }
    }

    #[test]
    fn shapes_are_checked_not_guessed() {
        let skills = built_in_skill_registry();
        let refused =
            |form: &str, payload: Value| structured_output(form, None, payload, &skills).is_err();
        assert!(refused(
            "table",
            json!({"columns": [{"label": "A", "numeric": false}], "rows": [["x"]]})
        ));
        assert!(refused(
            "table",
            json!({"columns": [{"label": "A", "numeric": false},
            {"label": "B", "numeric": false}], "rows": [["x"]]})
        ));
        assert!(refused(
            "calendar",
            json!({"events": [{"title": "X", "date": "15 Oct"}]})
        ));
        assert!(refused(
            "calendar",
            json!({"events": [{"title": "X", "date": "2026-10-15", "start": "9am"}]})
        ));
        assert!(refused(
            "comparison",
            json!({"options": [{"name": "Only"}]})
        ));
        assert!(refused(
            "map",
            json!({"connect": "route", "places": [{"label": "A", "latitude": 1, "longitude": 2}]})
        ));
        assert!(refused(
            "map",
            json!({"connect": "none", "places": [{"label": "A", "latitude": 91, "longitude": 2}]})
        ));
        assert!(refused(
            "map",
            json!({"connect": "none", "places": [{"label": "A", "latitude": "1", "longitude": 2}]})
        ));
        assert!(refused(
            "media",
            json!({"format": "gif", "url": "u", "alt": "a"})
        ));
        assert!(refused(
            "chart",
            json!({"chart_type": "pie", "accessible_summary": "s",
            "series": [{"name": "s", "points": [{"x": 1, "y": 1}]}]})
        ));
    }

    #[test]
    fn a_kind_no_skill_provides_is_refused() {
        let error = parse_fragment(r#"{"semantic_type": "recipe", "payload": {}}"#).unwrap_err();
        assert_eq!(error, SkillError::UnknownType("recipe".into()));
    }

    #[test]
    fn a_pack_kind_is_checked_as_its_block() {
        let skills = built_in_skill_registry();
        let output =
            structured_output("at_a_glance", Some("stats"), sample("stats"), &skills).unwrap();
        assert_eq!(
            (output.semantic_type.as_str(), output.block.as_str()),
            ("at_a_glance", "stats")
        );
        assert!(structured_output("at_a_glance", Some("stats"), sample("list"), &skills).is_err());
    }

    #[test]
    fn a_plugin_kind_is_held_to_its_schema() {
        let mut skills = built_in_skill_registry();
        skills
            .register(PresentationSkillManifest {
                id: "acme".into(),
                version: "1.0.0".into(),
                api: PRESENTATION_SKILL_API.into(),
                provides: vec!["acme.badge".into()],
                schema: Some(json!({"type": "object", "required": ["name"]})),
            })
            .unwrap();
        let ok = structured_output("acme.badge", None, json!({"name": "N"}), &skills).unwrap();
        assert_eq!(ok.block, "");
        assert!(structured_output("acme.badge", None, json!({}), &skills).is_err());
        let claims_form = skills.register(PresentationSkillManifest {
            id: "evil".into(),
            version: "1.0.0".into(),
            api: PRESENTATION_SKILL_API.into(),
            provides: vec!["table".into()],
            schema: None,
        });
        assert!(claims_form.is_err());
    }

    #[test]
    fn fences_are_found_in_order_and_projected_in_place() {
        let text = "Intro\n\n```vak\n{\"semantic_type\":\"callout\",\"payload\":{\"tone\":\"tip\",\"text\":\"Hi\"}}\n```\n\nOutro";
        assert_eq!(structured_outputs_from_text(text).len(), 1);
        let projected = project_structured_fences(text);
        assert!(
            projected.contains("> Hi")
                && projected.starts_with("Intro")
                && projected.ends_with("Outro")
        );
        let bare = r#"{"semantic_type":"callout","payload":{"tone":"tip","text":"Hi"}}"#;
        assert!(
            structured_outputs_from_text(bare).is_empty(),
            "JSON outside a fence is prose"
        );
    }
}
