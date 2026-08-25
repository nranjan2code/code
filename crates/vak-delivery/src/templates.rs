//! Declarative output template files and channel profile preferences.

use crate::{
    DeliveryError, Markup, TemplateActivation, TemplateNode, TemplateOrigin, TemplateRegistry,
    TemplateSlot, TemplateSpec,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct LoadedTemplates {
    pub registry: TemplateRegistry,
    pub channels: BTreeMap<String, ChannelPreference>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChannelPreference {
    pub template: Option<String>,
    pub markup: Option<Markup>,
    pub max_chars: Option<usize>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TemplateFile {
    templates: BTreeMap<String, TemplateDefinition>,
    channels: BTreeMap<String, ChannelPreference>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Deserialize)]
struct TemplateDefinition {
    #[serde(default = "default_revision")]
    revision: u32,
    format: String,
    #[serde(default)]
    activation: TemplateActivation,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

fn default_revision() -> u32 {
    1
}

/// Load user and project layers. Project files are ignored unless the caller
/// has already established project trust.
pub fn load_layers(
    user_path: &Path,
    project_path: &Path,
    project_trusted: bool,
) -> LoadedTemplates {
    let mut loaded = LoadedTemplates::default();
    if project_trusted {
        load_file(project_path, TemplateOrigin::Project, &mut loaded);
    } else if project_path.exists() {
        loaded.warnings.push(format!(
            "ignored untrusted project output templates at {}",
            project_path.display()
        ));
    }
    load_file(user_path, TemplateOrigin::User, &mut loaded);
    loaded
}

fn load_file(path: &Path, origin: TemplateOrigin, loaded: &mut LoadedTemplates) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            loaded
                .warnings
                .push(format!("could not read {}: {error}", path.display()));
            return;
        }
    };
    let parsed: TemplateFile = match toml::from_str(&text) {
        Ok(parsed) => parsed,
        Err(error) => {
            loaded.warnings.push(format!(
                "invalid output template {}: {error}",
                path.display()
            ));
            return;
        }
    };
    for key in parsed.unknown.keys() {
        loaded.warnings.push(format!(
            "ignored unknown output key {key} in {}",
            path.display()
        ));
    }
    for (id, definition) in parsed.templates {
        for key in definition.unknown.keys() {
            loaded.warnings.push(format!(
                "ignored unknown template key {id}.{key} in {}",
                path.display()
            ));
        }
        match compile_template(id.clone(), definition, origin) {
            Ok(template) => {
                if let Err(error) = loaded.registry.upsert(template) {
                    loaded.warnings.push(format!(
                        "invalid template {id} in {}: {error}",
                        path.display()
                    ));
                }
            }
            Err(error) => loaded.warnings.push(format!(
                "invalid template {id} in {}: {error}",
                path.display()
            )),
        }
    }
    for (surface, preference) in parsed.channels {
        for key in preference.unknown.keys() {
            loaded.warnings.push(format!(
                "ignored unknown channel key {surface}.{key} in {}",
                path.display()
            ));
        }
        if surface.trim().is_empty() {
            loaded
                .warnings
                .push(format!("empty channel name in {}", path.display()));
            continue;
        }
        if preference.max_chars == Some(0) {
            loaded.warnings.push(format!(
                "channel {surface} in {} has max_chars = 0",
                path.display()
            ));
            continue;
        }
        loaded.channels.insert(surface, preference);
    }
}

fn compile_template(
    id: String,
    definition: TemplateDefinition,
    origin: TemplateOrigin,
) -> Result<TemplateSpec, DeliveryError> {
    if definition.format.chars().count() > 32_768 {
        return Err(DeliveryError::InvalidTemplate(
            "format exceeds 32768 characters".into(),
        ));
    }
    let template = TemplateSpec {
        id,
        revision: definition.revision,
        origin,
        activation: definition.activation,
        nodes: parse_format(&definition.format)?,
    };
    template.validate()?;
    Ok(template)
}

fn parse_format(format: &str) -> Result<Vec<TemplateNode>, DeliveryError> {
    let mut nodes = Vec::new();
    let mut literal = String::new();
    let mut chars = format.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '{' && chars.peek() == Some(&'{') {
            chars.next();
            literal.push('{');
            continue;
        }
        if character == '}' && chars.peek() == Some(&'}') {
            chars.next();
            literal.push('}');
            continue;
        }
        if character != '{' {
            literal.push(character);
            continue;
        }
        if !literal.is_empty() {
            nodes.push(TemplateNode::Literal {
                text: std::mem::take(&mut literal),
            });
        }
        let mut name = String::new();
        loop {
            match chars.next() {
                Some('}') => break,
                Some('{') | None => {
                    return Err(DeliveryError::InvalidTemplate(
                        "unclosed or nested placeholder".into(),
                    ));
                }
                Some(value) => name.push(value),
            }
        }
        nodes.push(TemplateNode::Slot {
            slot: parse_slot(name.trim())?,
        });
    }
    if !literal.is_empty() {
        nodes.push(TemplateNode::Literal { text: literal });
    }
    Ok(nodes)
}

fn parse_slot(name: &str) -> Result<TemplateSlot, DeliveryError> {
    match name {
        "title" => Ok(TemplateSlot::Title),
        "body" => Ok(TemplateSlot::Body),
        "source_markdown" => Ok(TemplateSlot::SourceMarkdown),
        "block_count" => Ok(TemplateSlot::BlockCount),
        _ => name
            .strip_prefix("metadata.")
            .filter(|key| !key.is_empty())
            .map(|key| TemplateSlot::Metadata { key: key.into() })
            .ok_or_else(|| {
                DeliveryError::InvalidTemplate(format!("unknown placeholder {{{name}}}"))
            }),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn loads_safe_layer_and_rejects_unknown_placeholder() {
        let root =
            std::env::temp_dir().join(format!("vak-delivery-template-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("root");
        let user = root.join("output.toml");
        std::fs::write(
            &user,
            "[templates.compact]\nformat = \"{title}\\n\\n{body}\"\n\n[channels.telegram]\ntemplate = \"compact\"\nmax_chars = 4000\n",
        )
        .expect("template");
        let loaded = load_layers(&user, &root.join("project.toml"), true);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert!(loaded.registry.resolve("compact").is_some());
        assert_eq!(loaded.channels["telegram"].max_chars, Some(4000));

        std::fs::write(&user, "[templates.bad]\nformat = \"{execute}\"\n").expect("bad template");
        let loaded = load_layers(&user, &root.join("project.toml"), true);
        assert_eq!(loaded.warnings.len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }
}
