//! Delivery boundary for declarative adaptive presentations.
//!
//! This adapter deliberately keeps the existing Markdown/document packet as
//! the lossless fallback. Rich data is an optional projection of an already
//! authorized structured result.

use serde_json::Value;
use vak_presentation::{
    CompileInput, CompiledPresentation, LibraryScope, PresentationLibrary, Primitive, RenderNode,
};

/// Lower a compiled presentation to Markdown for surfaces that cannot draw
/// the render tree: a form node by its form's own text form, layout by its
/// parts, and the compiler's exact fallback when compilation degraded.
pub fn markdown(compiled: &CompiledPresentation) -> String {
    match compiled {
        CompiledPresentation::Fallback { text, .. } => text.clone(),
        CompiledPresentation::Rich(tree) => {
            let mut parts = Vec::new();
            render_node(&tree.root, &mut parts);
            parts.join("\n\n")
        }
    }
}

fn render_node(node: &RenderNode, parts: &mut Vec<String>) {
    let text = |key: &str| {
        node.props
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    };
    match node.primitive {
        Primitive::Stack | Primitive::Row | Primitive::Group => {}
        Primitive::Section => {
            let heading = [text("label"), text("title")]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            if !heading.is_empty() {
                parts.push(format!("**{heading}**"));
            }
            if !text("text").is_empty() {
                parts.push(text("text"));
            }
        }
        Primitive::Text => {
            if !text("text").is_empty() {
                parts.push(text("text"));
            }
        }
        Primitive::Metric => {
            let value = node.props.get("value").map(|value| match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            });
            let reading = [value.unwrap_or_default(), text("unit")]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            parts.push(format!("- {}: {reading}", text("label")));
        }
        form => {
            let output = crate::skills::StructuredOutput {
                semantic_type: form_name(form),
                block: form_name(form),
                schema_version: crate::PRESENTATION_SCHEMA_VERSION,
                skill_id: String::new(),
                skill_version: String::new(),
                payload: Value::Object(node.props.clone().into_iter().collect()),
            };
            parts.push(crate::skills::structured_markdown(&output));
        }
    }
    for child in &node.children {
        render_node(child, parts);
    }
}

/// A form primitive's name, as `vak_delivery::FORMS` spells it.
fn form_name(primitive: Primitive) -> String {
    serde_json::to_value(primitive)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

pub fn project(
    library: &PresentationLibrary,
    semantic_type: &str,
    payload: Value,
    fallback_text: impl Into<String>,
    scope: LibraryScope,
    owner: &str,
) -> Option<CompiledPresentation> {
    let stored = library.select(semantic_type, scope, owner)?;
    Some(vak_presentation::compile(
        &stored.spec,
        &CompileInput {
            semantic_type: semantic_type.to_owned(),
            payload,
            fallback_text: fallback_text.into(),
        },
    ))
}

pub fn project_preferred(
    library: &PresentationLibrary,
    semantic_type: &str,
    payload: Value,
    fallback_text: impl Into<String>,
    user_owner: &str,
    workspace_owner: &str,
) -> Option<CompiledPresentation> {
    let stored = library.select_preferred(semantic_type, user_owner, workspace_owner)?;
    Some(vak_presentation::compile(
        &stored.spec,
        &CompileInput {
            semantic_type: semantic_type.to_owned(),
            payload,
            fallback_text: fallback_text.into(),
        },
    ))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use vak_presentation::{
        LibraryScope, PresentationLibrary, PresentationOrigin, StoredPresentation, digest,
    };

    fn library() -> PresentationLibrary {
        let spec: vak_presentation::PresentationSpec = serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "id": "demo.timeline",
            "revision": 1,
            "accepts": ["demo"],
            "root": { "primitive": "text", "props": { "text": { "kind": "binding", "path": "$.title" } }, "children": [] }
        }))
        .expect("spec");
        let mut library = PresentationLibrary::default();
        library
            .register(StoredPresentation {
                digest: digest(&spec).expect("digest"),
                spec,
                origin: PresentationOrigin {
                    scope: LibraryScope::Workspace,
                    owner: "workspace".into(),
                    plugin_id: None,
                    generation: None,
                },
                enabled: false,
            })
            .expect("register");
        library
    }

    #[test]
    fn project_requires_explicit_activation_and_compiles_after_selection() {
        let mut library = library();
        assert!(
            project(
                &library,
                "demo",
                serde_json::json!({"title":"Preview"}),
                "Original",
                LibraryScope::Workspace,
                "workspace"
            )
            .is_none()
        );
        library
            .activate("demo.timeline", 1, LibraryScope::Workspace, "workspace")
            .expect("activate");
        let result = project(
            &library,
            "demo",
            serde_json::json!({"title":"Ready"}),
            "Original",
            LibraryScope::Workspace,
            "workspace",
        )
        .expect("projection");
        assert!(matches!(result, CompiledPresentation::Rich(_)));
    }

    #[test]
    fn markdown_lowering_is_generic_and_preserves_compiler_fallback() {
        let mut library = library();
        assert_eq!(
            markdown(&CompiledPresentation::Fallback {
                text: "Original answer".into(),
                reason: "invalid payload".into(),
            }),
            "Original answer"
        );
        library
            .activate("demo.timeline", 1, LibraryScope::Workspace, "workspace")
            .expect("activate");
        let compiled = project(
            &library,
            "demo",
            serde_json::json!({"title":"Ready"}),
            "Original",
            LibraryScope::Workspace,
            "workspace",
        )
        .expect("projection");
        assert!(markdown(&compiled).contains("Ready"));
    }
}
