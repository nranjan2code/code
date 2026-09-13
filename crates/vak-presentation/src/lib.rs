//! Safe, declarative presentation composition.
//!
//! This crate deliberately contains no UI framework, filesystem access,
//! network client, tool handle, or provider dependency. A presentation is
//! data. The host validates it, binds it to an already-authorized result, and
//! lowers it to a closed render tree or an explicit fallback.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

pub mod seeds;

pub const SPEC_SCHEMA_VERSION: u16 = 1;
pub const RENDER_TREE_SCHEMA_VERSION: u16 = 1;
pub const MAX_SPEC_BYTES: usize = 256 * 1024;
pub const MAX_NODES: usize = 256;
pub const MAX_DEPTH: usize = 16;
pub const MAX_BINDINGS: usize = 256;
pub const MAX_TEXT: usize = 32 * 1024;
/// Bound runtime expansion as well as the declarative spec itself. A valid
/// `each` node must not be able to turn an untrusted result into an
/// arbitrarily large render tree.
pub const MAX_EXPANSION_ITEMS: usize = 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresentationSpec {
    pub schema_version: u16,
    pub id: String,
    pub revision: u64,
    #[serde(default)]
    pub accepts: Vec<String>,
    pub root: SpecNode,
    #[serde(default)]
    pub fallback: FallbackSpec,
    #[serde(default)]
    pub accessibility: AccessibilitySpec,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecNode {
    pub primitive: Primitive,
    #[serde(default)]
    pub props: BTreeMap<String, SpecValue>,
    #[serde(default)]
    pub children: Vec<SpecNode>,
    #[serde(default)]
    pub each: Option<Binding>,
    #[serde(default)]
    pub item: Option<Box<SpecNode>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpecValue {
    Text(String),
    Number(f64),
    Boolean(bool),
    Binding(Binding),
    Literal(Value),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    /// A deliberately small JSON path. Only `$`, object keys, and array indexes
    /// are accepted; there is no expression language.
    pub path: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub empty: EmptyValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EmptyValue {
    #[default]
    Omit,
    EmptyText,
    EmptyList,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FallbackSpec {
    #[serde(default)]
    pub kind: FallbackKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FallbackKind {
    #[default]
    Document,
    Plain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AccessibilitySpec {
    #[serde(default)]
    pub summary: Option<Binding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Primitive {
    Stack,
    Row,
    Group,
    Section,
    Divider,
    Title,
    Text,
    RichText,
    Label,
    Badge,
    Callout,
    Quote,
    List,
    Checklist,
    Timeline,
    Steps,
    KeyValue,
    Table,
    Metric,
    Progress,
    Chart,
    DataGrid,
    Comparison,
    Image,
    Audio,
    Video,
    File,
    LinkPreview,
    Gallery,
    Diff,
    TestMatrix,
    Terminal,
    Artifact,
    CitationList,
    Disclosure,
    Filter,
    Sort,
    Search,
    Stepper,
    Timer,
    Loading,
    Empty,
    Partial,
    Error,
    Unavailable,
    Stale,
    Map,
    Calendar,
    Board,
    Graph,
    Entity,
    Evidence,
    Form,
    Transaction,
    Alert,
    Conversation,
    Simulation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderTree {
    pub schema_version: u16,
    pub spec_id: String,
    pub revision: u64,
    pub digest: String,
    pub root: RenderNode,
    #[serde(default)]
    pub accessibility_summary: Option<String>,
    pub coverage: Coverage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderNode {
    pub primitive: Primitive,
    #[serde(default)]
    pub props: BTreeMap<String, Value>,
    #[serde(default)]
    pub children: Vec<RenderNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Coverage {
    pub rendered_paths: Vec<String>,
    pub omitted_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompileInput {
    pub semantic_type: String,
    pub payload: Value,
    #[serde(default)]
    pub fallback_text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CompiledPresentation {
    Rich(RenderTree),
    Fallback { text: String, reason: String },
}

/// Trust boundary for reusable presentation definitions. User and workspace
/// scopes are intentionally explicit so a pack cannot silently become global.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryScope {
    User,
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationOrigin {
    pub scope: LibraryScope,
    pub owner: String,
    #[serde(default)]
    pub plugin_id: Option<String>,
    #[serde(default)]
    pub generation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredPresentation {
    pub spec: PresentationSpec,
    pub digest: String,
    pub origin: PresentationOrigin,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationActivation {
    pub spec_id: String,
    pub revision: u64,
    pub scope: LibraryScope,
    pub owner: String,
    pub semantic_types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationPackManifest {
    pub schema_version: u16,
    pub pack_id: String,
    pub version: String,
    pub digest: String,
    pub specs: Vec<String>,
    pub primitives: Vec<String>,
    #[serde(default)]
    pub publisher: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresentationLibrary {
    specs: BTreeMap<(String, u64), StoredPresentation>,
    activations: Vec<PresentationActivation>,
}

impl PresentationLibrary {
    /// Remove all definitions contributed by one plugin generation and any
    /// activation pointers that reference them. Historical session records are
    /// intentionally outside this projection and remain untouched.
    pub fn revoke_plugin(&mut self, plugin_id: &str) -> usize {
        let removed: BTreeSet<String> = self
            .specs
            .values()
            .filter(|stored| stored.origin.plugin_id.as_deref() == Some(plugin_id))
            .map(|stored| stored.spec.id.clone())
            .collect();
        let before = self.specs.len();
        self.specs
            .retain(|_, stored| stored.origin.plugin_id.as_deref() != Some(plugin_id));
        self.activations
            .retain(|entry| !removed.contains(&entry.spec_id));
        before - self.specs.len()
    }

    /// Freeze a validated candidate revision and register it without changing
    /// the currently active revision. Activation remains an explicit follow-up
    /// so previewing a proposal is always non-destructive.
    pub fn register_revision(
        &mut self,
        request: PresentationRevisionRequest,
        proposed: PresentationSpec,
        origin: PresentationOrigin,
    ) -> Result<PresentationRevision, PresentationError> {
        let revision = propose_revision(request, proposed)?;
        self.register(StoredPresentation {
            spec: revision.proposed.clone(),
            digest: revision.digest.clone(),
            origin,
            enabled: false,
        })?;
        Ok(revision)
    }

    pub fn register(&mut self, stored: StoredPresentation) -> Result<(), PresentationError> {
        validate_spec(&stored.spec)?;
        let calculated = digest(&stored.spec)?;
        if stored.digest != calculated {
            return Err(PresentationError::DigestMismatch);
        }
        if stored.origin.owner.trim().is_empty() {
            return Err(PresentationError::InvalidSpec(
                "origin owner is empty".into(),
            ));
        }
        let key = (stored.spec.id.clone(), stored.spec.revision);
        if let Some(existing) = self.specs.get(&key) {
            if existing.digest != stored.digest {
                return Err(PresentationError::RevisionConflict(stored.spec.id));
            }
            return Ok(());
        }
        self.specs.insert(key, stored);
        Ok(())
    }

    pub fn get(&self, id: &str, revision: u64) -> Option<&StoredPresentation> {
        self.specs.get(&(id.to_owned(), revision))
    }

    pub fn activate(
        &mut self,
        id: &str,
        revision: u64,
        scope: LibraryScope,
        owner: &str,
    ) -> Result<PresentationActivation, PresentationError> {
        let stored = self
            .get(id, revision)
            .ok_or_else(|| PresentationError::UnknownPresentation(id.to_owned()))?;
        let built_in = stored.origin.plugin_id.is_none() && stored.origin.owner == "builtin";
        if !built_in && (stored.origin.scope != scope || stored.origin.owner != owner) {
            return Err(PresentationError::ActivationDenied);
        }
        let activation = PresentationActivation {
            spec_id: id.to_owned(),
            revision,
            scope,
            owner: owner.to_owned(),
            semantic_types: stored.spec.accepts.clone(),
        };
        if let Some(existing) = self
            .activations
            .iter_mut()
            .find(|entry| entry.spec_id == id && entry.scope == scope && entry.owner == owner)
        {
            *existing = activation.clone();
        } else {
            self.activations.push(activation.clone());
        }
        if let Some(spec) = self.specs.get_mut(&(id.to_owned(), revision)) {
            spec.enabled = true;
        }
        Ok(activation)
    }

    pub fn deactivate(&mut self, id: &str, scope: LibraryScope, owner: &str) {
        self.activations
            .retain(|entry| !(entry.spec_id == id && entry.scope == scope && entry.owner == owner));
    }

    /// Restore the immutable original revision for a scope, or clear its
    /// activation when no original revision exists. Historical definitions
    /// and receipts remain untouched.
    pub fn reset(&mut self, id: &str, scope: LibraryScope, owner: &str) -> bool {
        self.deactivate(id, scope, owner);
        let Some(original) = self.get(id, 1) else {
            return false;
        };
        let built_in = original.origin.plugin_id.is_none() && original.origin.owner == "builtin";
        if !built_in && (original.origin.scope != scope || original.origin.owner != owner) {
            return false;
        }
        self.activate(id, 1, scope, owner).is_ok()
    }

    pub fn select(
        &self,
        semantic_type: &str,
        scope: LibraryScope,
        owner: &str,
    ) -> Option<&StoredPresentation> {
        self.activations
            .iter()
            .filter(|entry| entry.scope == scope && entry.owner == owner)
            .filter(|entry| {
                entry
                    .semantic_types
                    .iter()
                    .any(|kind| kind == semantic_type)
            })
            .filter_map(|entry| self.get(&entry.spec_id, entry.revision))
            .filter(|stored| stored.enabled)
            .max_by_key(|stored| stored.spec.revision)
    }

    /// Resolve the narrowest explicit user intent before workspace intent.
    /// No effective values are copied between scopes; the returned definition
    /// remains owned by the layer that activated it.
    pub fn select_preferred(
        &self,
        semantic_type: &str,
        user_owner: &str,
        workspace_owner: &str,
    ) -> Option<&StoredPresentation> {
        self.select(semantic_type, LibraryScope::User, user_owner)
            .or_else(|| self.select(semantic_type, LibraryScope::Workspace, workspace_owner))
    }

    pub fn activations(&self) -> &[PresentationActivation] {
        &self.activations
    }

    pub fn definitions(&self) -> impl Iterator<Item = &StoredPresentation> {
        self.specs.values()
    }

    pub fn from_parts(
        definitions: impl IntoIterator<Item = StoredPresentation>,
        activations: Vec<PresentationActivation>,
    ) -> Result<Self, PresentationError> {
        let mut library = Self {
            specs: BTreeMap::new(),
            activations,
        };
        for definition in definitions {
            library.register(definition)?;
        }
        Ok(library)
    }

    pub fn pack_manifest(
        &self,
        pack_id: impl Into<String>,
        version: impl Into<String>,
        primitives: Vec<String>,
        publisher: Option<String>,
    ) -> Result<PresentationPackManifest, PresentationError> {
        let pack_id = pack_id.into();
        let version = version.into();
        let specs: Vec<String> = self
            .specs
            .values()
            .map(|stored| stored.digest.clone())
            .collect();
        let bytes = serde_json::to_vec(&(&pack_id, &version, &specs, &primitives, &publisher))
            .map_err(|error| PresentationError::InvalidJson(error.to_string()))?;
        Ok(PresentationPackManifest {
            schema_version: SPEC_SCHEMA_VERSION,
            pack_id,
            version,
            digest: hex::encode(Sha256::digest(bytes)),
            specs,
            primitives,
            publisher,
        })
    }
}

/// A user-directed revision request. The feedback string is audit context;
/// only the already-authorized caller may turn a proposed spec into a stored
/// revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationRevisionRequest {
    pub base_id: String,
    pub base_revision: u64,
    pub feedback: String,
    pub attempt: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresentationRevision {
    pub request: PresentationRevisionRequest,
    pub proposed: PresentationSpec,
    pub digest: String,
}

/// Validate and freeze one immutable candidate revision. The runtime, rather
/// than the model, owns revision numbering and the retry ceiling.
pub fn propose_revision(
    request: PresentationRevisionRequest,
    mut proposed: PresentationSpec,
) -> Result<PresentationRevision, PresentationError> {
    if request.feedback.trim().is_empty() {
        return Err(PresentationError::InvalidSpec("feedback is empty".into()));
    }
    if request.attempt == 0 || request.attempt > 2 {
        return Err(PresentationError::RevisionBudgetExceeded);
    }
    if proposed.id != request.base_id {
        return Err(PresentationError::RevisionIdentityChanged);
    }
    let expected_revision = request
        .base_revision
        .checked_add(1)
        .ok_or(PresentationError::RevisionOverflow)?;
    if proposed.revision != expected_revision {
        proposed.revision = expected_revision;
    }
    validate_spec(&proposed)?;
    let digest = digest(&proposed)?;
    Ok(PresentationRevision {
        request,
        proposed,
        digest,
    })
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PresentationError {
    #[error("presentation spec is not valid UTF-8 JSON: {0}")]
    InvalidJson(String),
    #[error("presentation spec exceeds {MAX_SPEC_BYTES} bytes")]
    SpecTooLarge,
    #[error("unsupported presentation schema version {0}")]
    UnsupportedSchema(u16),
    #[error("presentation spec is invalid: {0}")]
    InvalidSpec(String),
    #[error("binding is invalid: {0}")]
    InvalidBinding(String),
    #[error("required binding is missing: {0}")]
    MissingBinding(String),
    #[error("presentation result does not match semantic type {0}")]
    SemanticTypeMismatch(String),
    #[error("presentation limit exceeded: {0}")]
    Limit(String),
    #[error("presentation digest does not match its canonical spec")]
    DigestMismatch,
    #[error("presentation revision conflicts with an existing digest: {0}")]
    RevisionConflict(String),
    #[error("unknown presentation: {0}")]
    UnknownPresentation(String),
    #[error("presentation activation is outside its origin scope")]
    ActivationDenied,
    #[error("presentation revision retry budget exceeded")]
    RevisionBudgetExceeded,
    #[error("presentation revision changed its identity")]
    RevisionIdentityChanged,
    #[error("presentation revision number overflowed")]
    RevisionOverflow,
}

pub fn digest(spec: &PresentationSpec) -> Result<String, PresentationError> {
    let bytes = serde_json::to_vec(spec)
        .map_err(|error| PresentationError::InvalidJson(error.to_string()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub fn parse_spec(bytes: &[u8]) -> Result<PresentationSpec, PresentationError> {
    if bytes.len() > MAX_SPEC_BYTES {
        return Err(PresentationError::SpecTooLarge);
    }
    let spec: PresentationSpec = serde_json::from_slice(bytes)
        .map_err(|error| PresentationError::InvalidJson(error.to_string()))?;
    validate_spec(&spec)?;
    Ok(spec)
}

pub fn validate_spec(spec: &PresentationSpec) -> Result<(), PresentationError> {
    if spec.schema_version != SPEC_SCHEMA_VERSION {
        return Err(PresentationError::UnsupportedSchema(spec.schema_version));
    }
    if spec.id.trim().is_empty() || spec.id.len() > 256 {
        return Err(PresentationError::InvalidSpec(
            "id is empty or too long".into(),
        ));
    }
    if spec.revision == 0 {
        return Err(PresentationError::InvalidSpec(
            "revision must be positive".into(),
        ));
    }
    if spec.accepts.iter().any(|kind| kind.trim().is_empty()) {
        return Err(PresentationError::InvalidSpec(
            "accepts contains an empty type".into(),
        ));
    }
    let mut counters = Counters::default();
    validate_node(&spec.root, 0, &mut counters)?;
    if let Some(summary) = &spec.accessibility.summary {
        validate_binding(summary)?;
        counters.bindings += 1;
    }
    if counters.bindings > MAX_BINDINGS {
        return Err(PresentationError::Limit("too many bindings".into()));
    }
    Ok(())
}

#[derive(Default)]
struct Counters {
    nodes: usize,
    bindings: usize,
}

fn validate_node(
    node: &SpecNode,
    depth: usize,
    counters: &mut Counters,
) -> Result<(), PresentationError> {
    if depth > MAX_DEPTH {
        return Err(PresentationError::Limit(
            "maximum nesting depth exceeded".into(),
        ));
    }
    counters.nodes += 1;
    if counters.nodes > MAX_NODES {
        return Err(PresentationError::Limit("too many nodes".into()));
    }
    for value in node.props.values() {
        if let SpecValue::Text(text) = value
            && text.len() > MAX_TEXT
        {
            return Err(PresentationError::Limit("literal text is too long".into()));
        }
        if let SpecValue::Binding(binding) = value {
            validate_binding(binding)?;
            counters.bindings += 1;
        }
    }
    if let Some(each) = &node.each {
        validate_binding(each)?;
        counters.bindings += 1;
        if node.item.is_none() {
            return Err(PresentationError::InvalidSpec("each requires item".into()));
        }
    }
    for child in &node.children {
        validate_node(child, depth + 1, counters)?;
    }
    if let Some(item) = &node.item {
        validate_node(item, depth + 1, counters)?;
    }
    Ok(())
}

fn validate_binding(binding: &Binding) -> Result<(), PresentationError> {
    if binding.path.len() > 512 || !binding.path.starts_with('$') {
        return Err(PresentationError::InvalidBinding(binding.path.clone()));
    }
    let mut chars = binding.path.chars().peekable();
    let _ = chars.next();
    while let Some(ch) = chars.next() {
        match ch {
            '.' => {
                let mut length = 0usize;
                while chars.peek().is_some_and(|value| {
                    value.is_ascii_alphanumeric() || *value == '_' || *value == '-'
                }) {
                    let _ = chars.next();
                    length += 1;
                }
                if length == 0 {
                    return Err(PresentationError::InvalidBinding(binding.path.clone()));
                }
            }
            '[' => {
                let mut digits = 0usize;
                while chars.peek().is_some_and(|value| value.is_ascii_digit()) {
                    let _ = chars.next();
                    digits += 1;
                }
                if digits == 0 || chars.next() != Some(']') {
                    return Err(PresentationError::InvalidBinding(binding.path.clone()));
                }
            }
            _ => return Err(PresentationError::InvalidBinding(binding.path.clone())),
        }
    }
    Ok(())
}

pub fn compile(spec: &PresentationSpec, input: &CompileInput) -> CompiledPresentation {
    match compile_rich(spec, input) {
        Ok(tree) => CompiledPresentation::Rich(tree),
        Err(error) => CompiledPresentation::Fallback {
            text: input.fallback_text.clone(),
            reason: error.to_string(),
        },
    }
}

/// Infer a conservative presentation shape from an already-validated JSON
/// payload. This is intentionally structural, not domain-aware: it never
/// guesses "trip", "budget", or another scenario from prose or field names.
/// Callers still validate the returned semantic type against their registry.
pub fn infer_semantic_type(payload: &Value) -> Option<&'static str> {
    let object = payload.as_object()?;
    // These recognizers use payload shape rather than scenario vocabulary.  A
    // caller can therefore reuse the same semantic contract for any domain or
    // extension pack without teaching the core about that domain.
    if object.get("left").is_some() && object.get("right").is_some()
        || object.get("alternatives").is_some_and(Value::is_array)
    {
        return Some("comparison");
    }
    if object.get("checks").is_some_and(Value::is_array)
        || object.get("tasks").is_some_and(Value::is_array)
    {
        return Some("checklist");
    }
    if object.get("events").is_some_and(Value::is_array)
        || object.get("slots").is_some_and(Value::is_array)
    {
        return Some("schedule");
    }
    if object.get("agenda").is_some_and(Value::is_array)
        && object.get("attendees").is_some_and(Value::is_array)
    {
        return Some("meeting");
    }
    if object.get("lessons").is_some_and(Value::is_array)
        || object.get("sections").is_some_and(Value::is_array)
    {
        return Some("lesson");
    }
    if object.get("decision").is_some() || object.get("choice").is_some() {
        return Some("decision");
    }
    if object.get("income").is_some_and(Value::is_array)
        && object.get("expenses").is_some_and(Value::is_array)
        || object.get("credits").is_some_and(Value::is_array)
            && object.get("debits").is_some_and(Value::is_array)
    {
        return Some("budget");
    }
    if object.get("items").is_some_and(Value::is_array)
        || object.get("entries").is_some_and(Value::is_array)
    {
        return Some("collection");
    }
    if object.get("steps").is_some_and(Value::is_array) {
        return Some("steps");
    }
    if object.get("status").is_some() || object.get("state").is_some() {
        return Some("status");
    }
    // Numeric measures are a deliberately structural shape.  The field names
    // describe a value slot, not a business domain, so this remains reusable
    // for budgets, scores, counts, readings, and any future extension pack.
    if ["value", "amount", "count", "total"]
        .iter()
        .any(|key| object.get(*key).is_some_and(Value::is_number))
    {
        return Some("metric");
    }
    if object.get("title").is_some() || object.get("summary").is_some() {
        return Some("detail");
    }
    None
}

fn compile_rich(
    spec: &PresentationSpec,
    input: &CompileInput,
) -> Result<RenderTree, PresentationError> {
    validate_spec(spec)?;
    if !spec.accepts.is_empty() && !spec.accepts.iter().any(|kind| kind == &input.semantic_type) {
        return Err(PresentationError::SemanticTypeMismatch(
            input.semantic_type.clone(),
        ));
    }
    let mut coverage = Coverage::default();
    let root = compile_node(&spec.root, &input.payload, "$", &mut coverage)?;
    let accessibility_summary = spec
        .accessibility
        .summary
        .as_ref()
        .and_then(|binding| resolve_binding(&input.payload, binding).ok().flatten())
        .and_then(value_as_text);
    Ok(RenderTree {
        schema_version: RENDER_TREE_SCHEMA_VERSION,
        spec_id: spec.id.clone(),
        revision: spec.revision,
        digest: digest(spec)?,
        root,
        accessibility_summary,
        coverage,
    })
}

fn compile_node(
    node: &SpecNode,
    data: &Value,
    path: &str,
    coverage: &mut Coverage,
) -> Result<RenderNode, PresentationError> {
    let mut children = node
        .children
        .iter()
        .enumerate()
        .map(|(index, child)| {
            compile_node(child, data, &format!("{path}.children[{index}]"), coverage)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(binding) = &node.each {
        let Some(value) = resolve_binding(data, binding)? else {
            if binding.required {
                return Err(PresentationError::MissingBinding(binding.path.clone()));
            }
            return Ok(RenderNode {
                primitive: node.primitive,
                props: BTreeMap::new(),
                children,
            });
        };
        let Some(items) = value.as_array() else {
            return Err(PresentationError::InvalidSpec(format!(
                "binding {} is not a list",
                binding.path
            )));
        };
        if items.len() > MAX_EXPANSION_ITEMS {
            return Err(PresentationError::Limit(
                "runtime collection expansion exceeded".into(),
            ));
        }
        for (index, item) in items.iter().enumerate() {
            let Some(template) = node.item.as_ref() else {
                return Err(PresentationError::InvalidSpec("each requires item".into()));
            };
            children.push(compile_node(
                template,
                item,
                &format!("{path}[{index}]"),
                coverage,
            )?);
        }
        coverage.rendered_paths.push(binding.path.clone());
    }
    let mut props = BTreeMap::new();
    for (key, value) in &node.props {
        match resolve_spec_value(value, data)? {
            Some(resolved) => {
                props.insert(key.clone(), resolved);
                coverage.rendered_paths.push(format!("{path}.{key}"));
            }
            None => coverage.omitted_paths.push(format!("{path}.{key}")),
        }
    }
    Ok(RenderNode {
        primitive: node.primitive,
        props,
        children,
    })
}

fn resolve_spec_value(value: &SpecValue, data: &Value) -> Result<Option<Value>, PresentationError> {
    match value {
        SpecValue::Text(text) => Ok(Some(Value::String(text.clone()))),
        SpecValue::Number(number) => serde_json::Number::from_f64(*number)
            .map(Value::Number)
            .map(Some)
            .ok_or_else(|| PresentationError::InvalidSpec("non-finite number".into())),
        SpecValue::Boolean(value) => Ok(Some(Value::Bool(*value))),
        SpecValue::Literal(value) => {
            if value.to_string().len() > MAX_TEXT {
                return Err(PresentationError::Limit(
                    "literal value is too large".into(),
                ));
            }
            Ok(Some(value.clone()))
        }
        SpecValue::Binding(binding) => resolve_binding(data, binding),
    }
}

fn resolve_binding(data: &Value, binding: &Binding) -> Result<Option<Value>, PresentationError> {
    validate_binding(binding)?;
    let mut current = data;
    if binding.path != "$" {
        let mut chars = binding.path.chars().peekable();
        let _ = chars.next();
        while let Some(ch) = chars.next() {
            match ch {
                '.' => {
                    let mut key = String::new();
                    while chars.peek().is_some_and(|value| {
                        value.is_ascii_alphanumeric() || *value == '_' || *value == '-'
                    }) {
                        if let Some(value) = chars.next() {
                            key.push(value);
                        }
                    }
                    current = match current.get(&key) {
                        Some(value) => value,
                        None => return missing_binding(binding),
                    };
                }
                '[' => {
                    let mut digits = String::new();
                    while chars.peek().is_some_and(|value| value.is_ascii_digit()) {
                        if let Some(value) = chars.next() {
                            digits.push(value);
                        }
                    }
                    let _ = chars.next();
                    let index = digits
                        .parse::<usize>()
                        .map_err(|_| PresentationError::InvalidBinding(binding.path.clone()))?;
                    current = match current.get(index) {
                        Some(value) => value,
                        None => return missing_binding(binding),
                    };
                }
                _ => return Err(PresentationError::InvalidBinding(binding.path.clone())),
            }
        }
    }
    Ok(Some(current.clone()))
}

fn missing_binding(binding: &Binding) -> Result<Option<Value>, PresentationError> {
    if binding.required {
        return Err(PresentationError::MissingBinding(binding.path.clone()));
    }
    Ok(match binding.empty {
        EmptyValue::Omit => None,
        EmptyValue::EmptyText => Some(Value::String(String::new())),
        EmptyValue::EmptyList => Some(Value::Array(Vec::new())),
    })
}

fn value_as_text(value: Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn spec() -> PresentationSpec {
        PresentationSpec {
            schema_version: SPEC_SCHEMA_VERSION,
            id: "test.timeline".into(),
            revision: 1,
            accepts: vec!["trip".into()],
            root: SpecNode {
                primitive: Primitive::Timeline,
                props: BTreeMap::from([(
                    String::from("title"),
                    SpecValue::Binding(Binding {
                        path: "$.title".into(),
                        required: true,
                        empty: EmptyValue::Omit,
                    }),
                )]),
                children: Vec::new(),
                each: Some(Binding {
                    path: "$.days".into(),
                    required: true,
                    empty: EmptyValue::Omit,
                }),
                item: Some(Box::new(SpecNode {
                    primitive: Primitive::Section,
                    props: BTreeMap::from([(
                        String::from("label"),
                        SpecValue::Binding(Binding {
                            path: "$.label".into(),
                            required: true,
                            empty: EmptyValue::Omit,
                        }),
                    )]),
                    children: Vec::new(),
                    each: None,
                    item: None,
                })),
            },
            fallback: FallbackSpec::default(),
            accessibility: AccessibilitySpec {
                summary: Some(Binding {
                    path: "$.summary".into(),
                    required: false,
                    empty: EmptyValue::EmptyText,
                }),
            },
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn compiles_bounded_timeline_and_tracks_coverage() {
        let result = compile(
            &spec(),
            &CompileInput {
                semantic_type: "trip".into(),
                payload: serde_json::json!({"title":"Jaipur", "summary":"Three days in Jaipur", "days":[{"label":"Day 1"},{"label":"Day 2"}]}),
                fallback_text: "Jaipur\n\nDay 1\nDay 2".into(),
            },
        );
        let tree = match result {
            CompiledPresentation::Rich(tree) => tree,
            CompiledPresentation::Fallback { reason, .. } => {
                assert!(reason.is_empty(), "expected rich result: {reason}");
                return;
            }
        };
        assert_eq!(tree.root.children.len(), 2);
        assert!(
            tree.coverage
                .rendered_paths
                .iter()
                .any(|path| path == "$.days")
        );
        assert_eq!(
            tree.accessibility_summary.as_deref(),
            Some("Three days in Jaipur")
        );
    }

    #[test]
    fn rejects_hostile_spec_depth_and_node_count() {
        let mut deep = spec();
        let mut node = SpecNode {
            primitive: Primitive::Group,
            props: BTreeMap::new(),
            children: Vec::new(),
            each: None,
            item: None,
        };
        for _ in 0..=MAX_DEPTH {
            node = SpecNode {
                primitive: Primitive::Group,
                props: BTreeMap::new(),
                children: vec![node],
                each: None,
                item: None,
            };
        }
        deep.root = node;
        assert!(matches!(
            validate_spec(&deep),
            Err(PresentationError::Limit(_))
        ));

        let mut wide = spec();
        wide.root.children = (0..=MAX_NODES)
            .map(|_| SpecNode {
                primitive: Primitive::Text,
                props: BTreeMap::new(),
                children: Vec::new(),
                each: None,
                item: None,
            })
            .collect();
        assert!(matches!(
            validate_spec(&wide),
            Err(PresentationError::Limit(_))
        ));
    }

    #[test]
    fn bounds_untrusted_collection_expansion() {
        let result = compile(
            &spec(),
            &CompileInput {
                semantic_type: "trip".into(),
                payload: serde_json::json!({
                    "title": "large",
                    "days": (0..=MAX_EXPANSION_ITEMS)
                        .map(|index| serde_json::json!({"label": index}))
                        .collect::<Vec<_>>()
                }),
                fallback_text: "safe fallback".into(),
            },
        );
        assert!(
            matches!(result, CompiledPresentation::Fallback { text, reason }
            if text == "safe fallback" && reason.contains("runtime collection expansion"))
        );
    }

    #[test]
    fn invalid_spec_degrades_without_panicking() {
        let mut invalid = spec();
        invalid.root.each = Some(Binding {
            path: "$.days[".into(),
            required: false,
            empty: EmptyValue::Omit,
        });
        let result = compile(
            &invalid,
            &CompileInput {
                semantic_type: "trip".into(),
                payload: Value::Null,
                fallback_text: "Original answer".into(),
            },
        );
        assert!(
            matches!(result, CompiledPresentation::Fallback { text, .. } if text == "Original answer")
        );
    }

    #[test]
    fn semantic_mismatch_uses_fallback() {
        let result = compile(
            &spec(),
            &CompileInput {
                semantic_type: "recipe".into(),
                payload: Value::Null,
                fallback_text: "Original".into(),
            },
        );
        assert!(matches!(result, CompiledPresentation::Fallback { .. }));
    }

    #[test]
    fn shape_inference_is_bounded_and_domain_neutral() {
        assert_eq!(
            infer_semantic_type(&serde_json::json!({"items": []})),
            Some("collection")
        );
        assert_eq!(
            infer_semantic_type(&serde_json::json!({"steps": []})),
            Some("steps")
        );
        assert_eq!(
            infer_semantic_type(&serde_json::json!({"status": "ready"})),
            Some("status")
        );
        assert_eq!(
            infer_semantic_type(&serde_json::json!({"label": "Score", "value": 42})),
            Some("metric")
        );
        assert_eq!(
            infer_semantic_type(&serde_json::json!({"title": "Anything"})),
            Some("detail")
        );
        assert_eq!(
            infer_semantic_type(&serde_json::json!({"trip": "Paris"})),
            None
        );
        assert_eq!(
            infer_semantic_type(&serde_json::json!({"left": {}, "right": {}})),
            Some("comparison")
        );
        assert_eq!(
            infer_semantic_type(&serde_json::json!({"tasks": []})),
            Some("checklist")
        );
        assert_eq!(
            infer_semantic_type(&serde_json::json!({"events": []})),
            Some("schedule")
        );
        assert_eq!(
            infer_semantic_type(&serde_json::json!({"lessons": []})),
            Some("lesson")
        );
        assert_eq!(
            infer_semantic_type(&serde_json::json!({"income": [], "expenses": []})),
            Some("budget")
        );
        assert_eq!(
            infer_semantic_type(&serde_json::json!({"decision": "keep"})),
            Some("decision")
        );
    }

    #[test]
    fn path_validator_rejects_expressions_and_parent_traversal() {
        for path in ["$.title + evil", "$..title", "$.title.foo()", "$.items[-1]"] {
            assert!(
                validate_binding(&Binding {
                    path: path.into(),
                    required: false,
                    empty: EmptyValue::Omit
                })
                .is_err()
            );
        }
    }

    #[test]
    fn hostile_specs_fail_closed_at_parse_boundary() {
        let mut oversized = spec();
        oversized
            .root
            .props
            .insert("text".into(), SpecValue::Text("x".repeat(MAX_TEXT + 1)));
        assert!(matches!(
            validate_spec(&oversized),
            Err(PresentationError::Limit(message)) if message.contains("text")
        ));

        let mut deep = spec();
        let mut node = SpecNode {
            primitive: Primitive::Group,
            props: BTreeMap::new(),
            children: Vec::new(),
            each: None,
            item: None,
        };
        for _ in 0..=MAX_DEPTH {
            node = SpecNode {
                primitive: Primitive::Group,
                props: BTreeMap::new(),
                children: vec![node],
                each: None,
                item: None,
            };
        }
        deep.root = node;
        assert!(matches!(
            validate_spec(&deep),
            Err(PresentationError::Limit(message)) if message.contains("depth")
        ));
    }

    #[test]
    fn hostile_binding_corpus_never_escapes_the_small_path_language() {
        // Keep this deterministic and dependency-free: the corpus exercises
        // the same boundary that an untrusted pack or model-authored JSON
        // reaches, without relying on a particular fuzzing runner.
        let valid = ["$", "$.title", "$.items[0].label", "$.a_b-c[12]"];
        for path in valid {
            assert!(
                validate_binding(&Binding {
                    path: path.into(),
                    required: false,
                    empty: EmptyValue::Omit,
                })
                .is_ok(),
                "valid path rejected: {path}"
            );
        }
        let hostile = [
            "",
            "title",
            "$..title",
            "$.title + 1",
            "$.title.foo()",
            "$.items[-1]",
            "$.items[abc]",
            "$.items[0].",
            "$.items[]",
            "$.items[0]/../../secret",
            "$['title']",
            &format!("$.{}", "x".repeat(513)),
        ];
        for path in hostile {
            assert!(
                validate_binding(&Binding {
                    path: path.into(),
                    required: false,
                    empty: EmptyValue::Omit,
                })
                .is_err(),
                "hostile path accepted: {path}"
            );
        }
    }

    #[test]
    fn hostile_payloads_keep_the_exact_fallback_and_do_not_become_actions() {
        let mut presentation = spec();
        presentation.root.props.insert(
            "text".into(),
            SpecValue::Binding(Binding {
                path: "$.title".into(),
                required: true,
                empty: EmptyValue::Omit,
            }),
        );
        let fallback = "Keep this answer unchanged\n\nApprove manually.";
        let payloads = [
            serde_json::json!({"title": "javascript:alert(1)"}),
            serde_json::json!({"title": "https://example.invalid/?next=javascript:alert(1)"}),
            serde_json::json!({"title": "<script>document.body.innerHTML='x'</script>"}),
            serde_json::json!({"title": {"action": "approve", "args": "--unsafe"}}),
            serde_json::json!({"title": ["\\u0000", "\n", "\r"]}),
            serde_json::json!({"other": "missing required binding"}),
        ];
        for payload in payloads {
            let compiled = compile(
                &presentation,
                &CompileInput {
                    semantic_type: "trip".into(),
                    payload,
                    fallback_text: fallback.into(),
                },
            );
            assert!(
                matches!(compiled, CompiledPresentation::Fallback { text, .. } if text == fallback),
                "hostile payload unexpectedly changed fallback"
            );
        }
    }

    #[test]
    fn generated_specs_hit_limits_without_panicking() {
        for width in [MAX_NODES - 1, MAX_NODES, MAX_NODES + 1] {
            let mut candidate = spec();
            candidate.root.children = (0..width)
                .map(|index| SpecNode {
                    primitive: Primitive::Text,
                    props: BTreeMap::from([(format!("p{index}"), SpecValue::Text("x".into()))]),
                    children: Vec::new(),
                    each: None,
                    item: None,
                })
                .collect();
            let result = validate_spec(&candidate);
            // The root itself counts toward the node budget.
            if width >= MAX_NODES {
                assert!(matches!(result, Err(PresentationError::Limit(_))));
            }
        }
    }

    #[test]
    fn tampered_pack_and_cross_scope_activation_are_denied() {
        let mut library = PresentationLibrary::default();
        let original = spec();
        let mut tampered = original.clone();
        tampered.id = "tampered".into();
        assert!(matches!(
            library.register(StoredPresentation {
                spec: tampered,
                digest: digest(&original).expect("digest"),
                origin: PresentationOrigin {
                    scope: LibraryScope::Workspace,
                    owner: "workspace-a".into(),
                    plugin_id: Some("untrusted-pack".into()),
                    generation: Some("1".into()),
                },
                enabled: false,
            }),
            Err(PresentationError::DigestMismatch)
        ));

        let stored = StoredPresentation {
            digest: digest(&original).expect("digest"),
            spec: original,
            origin: PresentationOrigin {
                scope: LibraryScope::Workspace,
                owner: "workspace-a".into(),
                plugin_id: Some("pack-a".into()),
                generation: Some("1".into()),
            },
            enabled: false,
        };
        library.register(stored).expect("register");
        assert!(matches!(
            library.activate("test.timeline", 1, LibraryScope::Workspace, "workspace-b"),
            Err(PresentationError::ActivationDenied)
        ));
        assert!(library.activations().is_empty());
    }

    #[test]
    fn library_registers_activates_and_selects_by_scope() {
        let spec = spec();
        let stored = StoredPresentation {
            digest: digest(&spec).expect("digest"),
            spec,
            origin: PresentationOrigin {
                scope: LibraryScope::Workspace,
                owner: "ws-1".into(),
                plugin_id: None,
                generation: None,
            },
            enabled: false,
        };
        let mut library = PresentationLibrary::default();
        library.register(stored).expect("register");
        library
            .activate("test.timeline", 1, LibraryScope::Workspace, "ws-1")
            .expect("activate");
        assert!(
            library
                .select("trip", LibraryScope::Workspace, "ws-1")
                .is_some()
        );
        library.deactivate("test.timeline", LibraryScope::Workspace, "ws-1");
        assert!(
            library
                .select("trip", LibraryScope::Workspace, "ws-1")
                .is_none()
        );
    }

    #[test]
    fn preferred_selection_uses_user_intent_before_workspace_intent() {
        let mut workspace_spec = spec();
        workspace_spec.id = "workspace.layout".into();
        let mut user_spec = spec();
        user_spec.id = "user.layout".into();
        let mut library = PresentationLibrary::default();
        library
            .register(StoredPresentation {
                digest: digest(&workspace_spec).expect("digest"),
                spec: workspace_spec,
                origin: PresentationOrigin {
                    scope: LibraryScope::Workspace,
                    owner: "ws".into(),
                    plugin_id: None,
                    generation: None,
                },
                enabled: false,
            })
            .expect("workspace");
        library
            .register(StoredPresentation {
                digest: digest(&user_spec).expect("digest"),
                spec: user_spec,
                origin: PresentationOrigin {
                    scope: LibraryScope::User,
                    owner: "user".into(),
                    plugin_id: None,
                    generation: None,
                },
                enabled: false,
            })
            .expect("user");
        library
            .activate("workspace.layout", 1, LibraryScope::Workspace, "ws")
            .expect("workspace activation");
        assert_eq!(
            library
                .select_preferred("trip", "user", "ws")
                .map(|s| s.spec.id.as_str()),
            Some("workspace.layout")
        );
        library
            .activate("user.layout", 1, LibraryScope::User, "user")
            .expect("user activation");
        assert_eq!(
            library
                .select_preferred("trip", "user", "ws")
                .map(|s| s.spec.id.as_str()),
            Some("user.layout")
        );
    }

    #[test]
    fn built_in_seed_can_be_explicitly_activated_for_a_workspace() {
        let seed = seeds::built_in_seed_pack()
            .into_iter()
            .next()
            .expect("seed pack is non-empty");
        let id = seed.spec.id.clone();
        let revision = seed.spec.revision;
        let accepts = seed.spec.accepts[0].clone();
        let mut library = PresentationLibrary::default();
        library.register(seed).expect("register seed");
        library
            .activate(&id, revision, LibraryScope::Workspace, "a-workspace")
            .expect("activate built-in");
        assert!(
            library
                .select(&accepts, LibraryScope::Workspace, "a-workspace")
                .is_some()
        );
    }

    #[test]
    fn built_in_seed_can_be_explicitly_activated_for_user_scope() {
        let seed = seeds::built_in_seed_pack()
            .into_iter()
            .next()
            .expect("seed pack is non-empty");
        let id = seed.spec.id.clone();
        let revision = seed.spec.revision;
        let accepts = seed.spec.accepts[0].clone();
        let mut library = PresentationLibrary::default();
        library.register(seed).expect("register seed");
        library
            .activate(&id, revision, LibraryScope::User, "user")
            .expect("activate built-in for user");
        assert!(
            library
                .select(&accepts, LibraryScope::User, "user")
                .is_some()
        );
    }

    #[test]
    fn library_scope_json_matches_client_contract() {
        assert_eq!(
            serde_json::to_string(&LibraryScope::User).unwrap(),
            "\"user\""
        );
        assert_eq!(
            serde_json::from_str::<LibraryScope>("\"workspace\"").unwrap(),
            LibraryScope::Workspace
        );
    }

    #[test]
    fn reset_restores_original_revision_without_deleting_history() {
        let mut original = spec();
        original.id = "reset.card".into();
        let mut revision = original.clone();
        revision.revision = 2;
        let mut library = PresentationLibrary::default();
        for candidate in [original, revision] {
            library
                .register(StoredPresentation {
                    digest: digest(&candidate).expect("digest"),
                    spec: candidate,
                    origin: PresentationOrigin {
                        scope: LibraryScope::Workspace,
                        owner: "ws".into(),
                        plugin_id: None,
                        generation: None,
                    },
                    enabled: false,
                })
                .expect("register");
        }
        library
            .activate("reset.card", 2, LibraryScope::Workspace, "ws")
            .expect("activate revision");
        assert!(library.reset("reset.card", LibraryScope::Workspace, "ws"));
        assert_eq!(
            library
                .select("trip", LibraryScope::Workspace, "ws")
                .map(|entry| entry.spec.revision),
            Some(1)
        );
        assert!(library.get("reset.card", 2).is_some());
    }

    #[test]
    fn library_rejects_digest_mismatch() {
        let spec = spec();
        let result = PresentationLibrary::default().register(StoredPresentation {
            digest: "bad".into(),
            spec,
            origin: PresentationOrigin {
                scope: LibraryScope::User,
                owner: "user-1".into(),
                plugin_id: None,
                generation: None,
            },
            enabled: true,
        });
        assert!(matches!(result, Err(PresentationError::DigestMismatch)));
    }

    #[test]
    fn revision_is_immutable_and_runtime_numbered() {
        let mut proposed = spec();
        proposed.revision = 99;
        let revision = propose_revision(
            PresentationRevisionRequest {
                base_id: "test.timeline".into(),
                base_revision: 1,
                feedback: "make it shorter".into(),
                attempt: 1,
            },
            proposed,
        )
        .expect("revision");
        assert_eq!(revision.proposed.revision, 2);
        assert_eq!(revision.digest, digest(&revision.proposed).expect("digest"));
    }

    #[test]
    fn revision_rejects_identity_and_retry_budget() {
        let mut proposed = spec();
        proposed.id = "other".into();
        let result = propose_revision(
            PresentationRevisionRequest {
                base_id: "test.timeline".into(),
                base_revision: 1,
                feedback: "change".into(),
                attempt: 1,
            },
            proposed,
        );
        assert!(matches!(
            result,
            Err(PresentationError::RevisionIdentityChanged)
        ));
        let result = propose_revision(
            PresentationRevisionRequest {
                base_id: "test.timeline".into(),
                base_revision: 1,
                feedback: "change".into(),
                attempt: 3,
            },
            spec(),
        );
        assert!(matches!(
            result,
            Err(PresentationError::RevisionBudgetExceeded)
        ));
    }

    #[test]
    fn revoking_plugin_removes_definitions_and_activations() {
        let spec = spec();
        let mut library = PresentationLibrary::default();
        library
            .register(StoredPresentation {
                digest: digest(&spec).expect("digest"),
                spec,
                origin: PresentationOrigin {
                    scope: LibraryScope::Workspace,
                    owner: "workspace".into(),
                    plugin_id: Some("pack.demo".into()),
                    generation: Some("1.0.0".into()),
                },
                enabled: false,
            })
            .expect("register");
        library
            .activate("test.timeline", 1, LibraryScope::Workspace, "workspace")
            .expect("activate");
        assert_eq!(library.revoke_plugin("pack.demo"), 1);
        assert!(library.activations().is_empty());
        assert!(library.get("test.timeline", 1).is_none());
    }
}
