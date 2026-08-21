use std::collections::{BTreeMap, VecDeque};

use crate::types::FlowDef;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("toml error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("flow has no nodes")]
    Empty,
    #[error("duplicate node id '{0}'")]
    Duplicate(String),
    #[error("node '{0}' has unknown type '{1}' (agent|bash|approval|merge)")]
    UnknownType(String, String),
    #[error("node '{0}' of type '{1}' is missing required field {2}")]
    MissingField(String, String, &'static str),
    #[error("node '{0}' depends on unknown node '{1}'")]
    UnknownDep(String, String),
    #[error("node '{0}' depends on itself")]
    SelfDep(String),
    #[error("cycle detected involving: {0}")]
    Cycle(String),
}

pub const NODE_TYPES: [&str; 4] = ["agent", "bash", "approval", "merge"];

#[derive(serde::Deserialize)]
struct FlowFile {
    flow: FlowHeader,
    #[serde(default)]
    nodes: Vec<crate::types::NodeDef>,
}

#[derive(serde::Deserialize)]
struct FlowHeader {
    name: String,
    #[serde(default)]
    description: String,
}

pub fn parse_flow(toml_str: &str) -> Result<FlowDef, ParseError> {
    let file: FlowFile = toml::from_str(toml_str)?;
    let mut flow = FlowDef {
        name: file.flow.name,
        description: file.flow.description,
        nodes: file.nodes,
    };
    validate(&mut flow)?;
    Ok(flow)
}

pub fn validate(flow: &mut FlowDef) -> Result<(), ParseError> {
    if flow.nodes.is_empty() {
        return Err(ParseError::Empty);
    }
    let mut ids = BTreeMap::new();
    for n in &flow.nodes {
        if ids.contains_key(&n.id) {
            return Err(ParseError::Duplicate(n.id.clone()));
        }
        if !NODE_TYPES.contains(&n.r#type.as_str()) {
            return Err(ParseError::UnknownType(n.id.clone(), n.r#type.clone()));
        }
        match n.r#type.as_str() {
            "agent" if n.prompt.is_none() => {
                return Err(ParseError::MissingField(
                    n.id.clone(),
                    "agent".into(),
                    "prompt",
                ));
            }
            "bash" if n.command.is_none() => {
                return Err(ParseError::MissingField(
                    n.id.clone(),
                    "bash".into(),
                    "command",
                ));
            }
            "approval" if n.message.is_none() => {
                return Err(ParseError::MissingField(
                    n.id.clone(),
                    "approval".into(),
                    "message",
                ));
            }
            _ => {}
        }
        ids.insert(n.id.clone(), ());
    }
    for n in &flow.nodes {
        for dep in &n.deps {
            if dep == &n.id {
                return Err(ParseError::SelfDep(n.id.clone()));
            }
            if !ids.contains_key(dep) {
                return Err(ParseError::UnknownDep(n.id.clone(), dep.clone()));
            }
        }
    }

    // Template references imply dependencies.
    let mut resolved: Vec<crate::types::NodeDef> = Vec::with_capacity(flow.nodes.len());
    for n in &flow.nodes {
        let mut node = n.clone();
        for (_, reference) in template_refs(&node) {
            if !ids.contains_key(&reference) {
                return Err(ParseError::UnknownDep(node.id.clone(), reference));
            }
            if reference != node.id && !node.deps.contains(&reference) {
                node.deps.push(reference);
            }
        }
        resolved.push(node);
    }
    flow.nodes = resolved;

    // Kahn's algorithm: cycle detection + layer assignment.
    let layers = layers(flow)?;
    let _ = layers;
    Ok(())
}

/// Extracts `{{identifier}}` references from a node's prompt or command.
fn template_refs(node: &crate::types::NodeDef) -> Vec<(String, String)> {
    let sources = [
        ("prompt", node.prompt.as_deref().unwrap_or_default()),
        ("command", node.command.as_deref().unwrap_or_default()),
    ];
    let mut out = Vec::new();
    for (field, text) in sources {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i + 3 < bytes.len() {
            if &text[i..i + 2] == "{{"
                && let Some(end_offset) = text[i + 2..].find("}}")
            {
                let inner = text[i + 2..i + 2 + end_offset].trim();
                if !inner.is_empty()
                    && inner
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                {
                    out.push((field.to_string(), inner.to_string()));
                }
                i += 2 + end_offset + 2;
            } else {
                i += 1;
            }
        }
    }
    out
}

/// Topological layers: nodes in the same layer have no mutual dependencies
/// and may run concurrently.
pub fn layers(flow: &FlowDef) -> Result<Vec<Vec<String>>, ParseError> {
    let mut indegree: BTreeMap<&str, usize> = BTreeMap::new();
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for n in &flow.nodes {
        indegree.entry(n.id.as_str()).or_insert(0);
        for dep in &n.deps {
            *indegree.entry(n.id.as_str()).or_insert(0) += 1;
            dependents
                .entry(dep.as_str())
                .or_default()
                .push(n.id.as_str());
        }
    }

    let mut queue: VecDeque<&str> = indegree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut layers: Vec<Vec<String>> = Vec::new();
    let mut placed = 0usize;

    while !queue.is_empty() {
        let mut next_queue = VecDeque::new();
        let mut layer = Vec::new();
        for id in queue.drain(..) {
            layer.push(id.to_string());
            placed += 1;
            if let Some(deps) = dependents.get(id) {
                for d in deps {
                    if let Some(e) = indegree.get_mut(*d) {
                        *e -= 1;
                        if *e == 0 {
                            next_queue.push_back(*d);
                        }
                    }
                }
            }
        }
        layers.push(layer);
        queue = next_queue;
    }

    if placed != flow.nodes.len() {
        let stuck: Vec<String> = indegree
            .iter()
            .filter(|(_, d)| **d > 0)
            .map(|(id, _)| id.to_string())
            .collect();
        return Err(ParseError::Cycle(stuck.join(", ")));
    }
    Ok(layers)
}
