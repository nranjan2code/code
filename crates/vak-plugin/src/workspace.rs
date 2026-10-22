//! Installable workspace contracts. Package descriptions are data, never host code.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSpec {
    pub schema: u32,
    pub id: String,
    pub title: String,
    pub description: String,
    pub initial: Value,
    pub state_schema: Value,
    pub fields: Vec<Field>,
    pub commands: Vec<Command>,
    /// JSON pointers explicitly admitted to an Agent context projection.
    pub context: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub path: String,
    pub label: String,
    pub kind: FieldKind,
    pub unit: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Text,
    Number,
    Boolean,
    Json,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub id: String,
    pub title: String,
    pub arguments: Value,
    pub edits: Vec<Edit>,
    /// Optional pure WebAssembly computation, with no host imports.
    pub compute: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub target: String,
    pub argument: String,
}

pub fn valid_pointer(path: &str) -> bool {
    path.len() <= 512
        && path.starts_with('/')
        && path.split('/').skip(1).all(|part| {
            !part.is_empty() && !["__proto__", "prototype", "constructor"].contains(&part) && {
                let mut chars = part.chars();
                let mut valid = true;
                while let Some(c) = chars.next() {
                    if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
                        valid = false;
                        break;
                    }
                }
                valid
            }
        })
}
fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
}
impl WorkspaceSpec {
    pub fn validate(&self, plugin: &str) -> Result<(), String> {
        let local = self
            .id
            .strip_prefix(&format!("{plugin}/"))
            .ok_or("workspace id must belong to its plugin")?;
        if self.schema != 1
            || !name(local)
            || self.title.is_empty()
            || self.title.len() > 160
            || self.description.len() > 4096
        {
            return Err("invalid workspace identity or schema".into());
        }
        if self.fields.len() > 128 || self.commands.len() > 128 || self.context.len() > 128 {
            return Err("workspace contract is too large".into());
        }
        schema(&self.state_schema, 0)?;
        vak_tools::validate_input(&self.state_schema, &self.initial)?;
        let mut paths = std::collections::BTreeSet::new();
        for field in &self.fields {
            if !valid_pointer(&field.path)
                || !paths.insert(&field.path)
                || field.label.is_empty()
                || field.label.len() > 160
                || self.initial.pointer(&field.path).is_none()
            {
                return Err("invalid workspace field".into());
            }
        }
        for field in &self.fields {
            let value = self
                .initial
                .pointer(&field.path)
                .ok_or("missing initial field")?;
            let valid = match field.kind {
                FieldKind::Text => value.is_string(),
                FieldKind::Number => value.is_number(),
                FieldKind::Boolean => value.is_boolean(),
                FieldKind::Json => true,
            };
            if !valid || field.unit.as_ref().is_some_and(|unit| unit.len() > 80) {
                return Err("field type does not match initial state".into());
            }
        }
        let mut names = std::collections::BTreeSet::new();
        for command in &self.commands {
            if !name(&command.id)
                || !names.insert(&command.id)
                || command.title.is_empty()
                || command.edits.is_empty()
                || command.edits.len() > 128
            {
                return Err("invalid workspace command".into());
            }
            schema(&command.arguments, 0)?;
            for edit in &command.edits {
                if !valid_pointer(&edit.target)
                    || !valid_pointer(&edit.argument)
                    || self.initial.pointer(&edit.target).is_none()
                {
                    return Err("invalid workspace edit address".into());
                }
            }
        }
        if self
            .context
            .iter()
            .any(|path| !valid_pointer(path) || self.initial.pointer(path).is_none())
        {
            return Err("invalid workspace context address".into());
        }
        Ok(())
    }
    pub fn apply(&self, state: &Value, command: &str, arguments: &Value) -> Result<Value, String> {
        vak_tools::validate_input(&self.state_schema, state)?;
        let command = self
            .commands
            .iter()
            .find(|item| item.id == command)
            .ok_or("unknown workspace command")?;
        vak_tools::validate_input(&command.arguments, arguments)?;
        let mut result = state.clone();
        for edit in &command.edits {
            let value = arguments
                .pointer(&edit.argument)
                .ok_or("missing command argument")?
                .clone();
            *result
                .pointer_mut(&edit.target)
                .ok_or("workspace target no longer exists")? = value;
        }
        vak_tools::validate_input(&self.state_schema, &result)?;
        Ok(result)
    }
    pub fn project(&self, state: &Value) -> Result<Value, String> {
        vak_tools::validate_input(&self.state_schema, state)?;
        self.context
            .iter()
            .map(|path| {
                Ok((
                    path.clone(),
                    state
                        .pointer(path)
                        .ok_or("context address no longer exists")?
                        .clone(),
                ))
            })
            .collect::<Result<serde_json::Map<String, Value>, String>>()
            .map(Value::Object)
    }
}

/// Closed, enforceable schema vocabulary: unsupported validation keywords fail,
/// rather than appear to constrain a command while the validator ignores them.
fn schema(value: &Value, depth: usize) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or("workspace schema must be an object")?;
    if depth > 24 {
        return Err("workspace schema is too deep".into());
    }
    for (key, child) in object {
        match key.as_str() {
            "type"
                if child.as_str().is_some_and(|kind| {
                    [
                        "object", "array", "string", "number", "integer", "boolean", "null",
                    ]
                    .contains(&kind)
                }) => {}
            "properties" => {
                for item in child
                    .as_object()
                    .ok_or("properties must be an object")?
                    .values()
                {
                    schema(item, depth + 1)?;
                }
            }
            "items" => schema(child, depth + 1)?,
            "required"
                if child
                    .as_array()
                    .is_some_and(|list| list.iter().all(Value::is_string)) => {}
            "additionalProperties" if child.is_boolean() => {}
            "enum" if child.as_array().is_some_and(|list| !list.is_empty()) => {}
            "description" if child.is_string() => {}
            _ => return Err(format!("unsupported workspace schema keyword: {key}")),
        }
    }
    if !object.contains_key("type") {
        return Err("workspace schema needs a type".into());
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    fn spec() -> WorkspaceSpec {
        serde_json::from_str(include_str!(
            "../../../examples/canvas-workspace/workspaces/parameters.json"
        ))
        .unwrap()
    }
    #[test]
    fn commands_are_typed_atomic_and_context_is_explicit() {
        let spec = spec();
        spec.validate("workspace-lab").unwrap();
        let next = spec
            .apply(
                &spec.initial,
                "set-parameters",
                &serde_json::json!({"name":"Stator","length":24,"enabled":false}),
            )
            .unwrap();
        assert_eq!(next["name"], "Stator");
        assert_eq!(spec.initial["name"], "Rotor");
        assert!(
            spec.apply(&next, "unknown", &serde_json::json!({}))
                .is_err()
        );
        assert!(
            spec.apply(
                &next,
                "set-parameters",
                &serde_json::json!({"name":"Oops","length":"24","enabled":false})
            )
            .is_err()
        );
        assert_eq!(spec.project(&next).unwrap()["/length"], 24);
    }
    #[test]
    fn namespaces_addresses_and_unenforced_schemas_are_refused() {
        let mut spec = spec();
        assert!(spec.validate("other-plugin").is_err());
        spec.context.push("/secret-that-is-not-a-resource".into());
        assert!(spec.validate("workspace-lab").is_err());
        let mut spec = super::tests::spec();
        spec.state_schema["minimum"] = serde_json::json!(0);
        assert!(spec.validate("workspace-lab").is_err());
        for pointer in ["", "name", "/__proto__/x", "/constructor/x", "/~3", "/a//b"] {
            assert!(!valid_pointer(pointer));
        }
        assert!(valid_pointer("/cells/a~1b"));
    }
    #[test]
    fn package_inventory_validates_before_install_and_activation_is_digest_bound() {
        let source = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(source.path().join("workspaces")).unwrap();
        std::fs::write(
            source.path().join("vak-plugin.json"),
            include_str!("../../../examples/canvas-workspace/vak-plugin.json"),
        )
        .unwrap();
        std::fs::write(
            source.path().join("workspaces/parameters.json"),
            serde_json::to_vec(&spec()).unwrap(),
        )
        .unwrap();
        let inspected = crate::inspect_package(source.path()).unwrap();
        assert_eq!(inspected.workspace_specs().unwrap().len(), 1);
        let store = crate::PluginStore::new(home.path());
        let plugin = store
            .install_local(source.path(), crate::InstallOptions::default())
            .unwrap();
        assert!(!plugin.enabled);
        store.enable(&plugin.name).unwrap();
        let generation = store.load().unwrap().generation;
        assert_eq!(
            store
                .with_enabled(&plugin.name, &plugin.digest, generation, || 7)
                .unwrap(),
            7
        );
        store.disable(&plugin.name).unwrap();
        let mut dispatched = false;
        assert!(
            store
                .with_enabled(&plugin.name, &plugin.digest, generation, || {
                    dispatched = true;
                })
                .is_err()
        );
        assert!(!dispatched);
        std::fs::write(
            source.path().join("workspaces/parameters.json"),
            "{\"schema\":1,\"script\":\"steal()\"}",
        )
        .unwrap();
        assert!(crate::inspect_package(source.path()).is_err());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod activation_barrier_tests {
    #[test]
    fn activation_writer_barrier_cannot_expire_under_a_live_commit() {
        let home = tempfile::tempdir().unwrap();
        let lock = crate::RegistryLock::acquire(home.path()).unwrap();
        let root = home.path().to_path_buf();
        let (sent, received) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let _second = crate::RegistryLock::acquire(&root).unwrap();
            sent.send(()).unwrap();
        });
        assert!(
            received
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_err()
        );
        drop(lock);
        received
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        thread.join().unwrap();
    }
}
