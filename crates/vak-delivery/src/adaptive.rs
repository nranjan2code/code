//! Delivery boundary for declarative adaptive presentations.
//!
//! This adapter deliberately keeps the existing Markdown/document packet as
//! the lossless fallback. Rich data is an optional projection of an already
//! authorized structured result.

use serde_json::Value;
use vak_presentation::{
    CompileInput, CompiledPresentation, LibraryScope, PresentationLibrary, Primitive, RenderNode,
};

/// Lower a compiled presentation to conservative Markdown for surfaces that
/// cannot consume the native render tree. This is deliberately generic: it
/// understands the host primitive vocabulary, never semantic/domain names,
/// and returns the compiler's exact fallback when compilation degraded.
pub fn markdown(compiled: &CompiledPresentation) -> String {
    match compiled {
        CompiledPresentation::Fallback { text, .. } => text.clone(),
        CompiledPresentation::Rich(tree) => render_node(&tree.root, 0),
    }
}

fn render_node(node: &RenderNode, depth: usize) -> String {
    let indent = "  ".repeat(depth.min(8));
    let text = |key: &str| node.props.get(key).and_then(|v| v.as_str()).unwrap_or("");
    let entries = |key: &str| entry_lines(node, key);
    let mut out = String::new();
    match node.primitive {
        Primitive::Title => out.push_str(&format!("{}# {}\n", indent, text("text"))),
        Primitive::Section => out.push_str(&format!("{}## {}\n", indent, text("title"))),
        Primitive::Label | Primitive::Badge => {
            out.push_str(&format!("{}{}\n", indent, text("text")))
        }
        Primitive::Divider => out.push_str(&format!("{}---\n", indent)),
        Primitive::Quote | Primitive::Callout => {
            let value = text("text");
            if !value.is_empty() {
                out.push_str(&format!("> {}\n", value));
            }
        }
        Primitive::List | Primitive::Checklist | Primitive::Timeline | Primitive::Steps => {
            let value = text("text");
            if !value.is_empty() {
                out.push_str(&format!("{}- {}\n", indent, value));
            }
        }
        Primitive::KeyValue | Primitive::Metric | Primitive::Progress => {
            let label = text("label");
            let value = text("value");
            if !label.is_empty() || !value.is_empty() {
                out.push_str(&format!("{}**{}:** {}\n", indent, label, value));
            }
        }
        Primitive::Text
        | Primitive::RichText
        | Primitive::Group
        | Primitive::Stack
        | Primitive::Row
        | Primitive::Disclosure
        | Primitive::Comparison
        | Primitive::Table
        | Primitive::Chart
        | Primitive::DataGrid
        | Primitive::Image
        | Primitive::Audio
        | Primitive::Video
        | Primitive::File
        | Primitive::LinkPreview
        | Primitive::Gallery
        | Primitive::Diff
        | Primitive::TestMatrix
        | Primitive::Terminal
        | Primitive::Artifact
        | Primitive::CitationList
        | Primitive::Filter
        | Primitive::Sort
        | Primitive::Search
        | Primitive::Stepper
        | Primitive::Timer
        | Primitive::Loading
        | Primitive::Empty
        | Primitive::Partial
        | Primitive::Error
        | Primitive::Unavailable
        | Primitive::Stale
        | Primitive::Map
        | Primitive::Calendar
        | Primitive::Board
        | Primitive::Graph
        | Primitive::Entity
        | Primitive::Evidence
        | Primitive::Form
        | Primitive::Transaction
        | Primitive::Alert
        | Primitive::Conversation
        | Primitive::Simulation => {
            let value = text("text");
            if !value.is_empty() {
                out.push_str(&format!("{}{}\n", indent, value));
            }
        }
        Primitive::Recipe => {
            let title = text("title");
            if !title.is_empty() {
                out.push_str(&format!("{}## {}\n", indent, title));
            }
            for (label, key) in [("Serves", "servings"), ("Time", "total_time")] {
                let value = text(key);
                if !value.is_empty() {
                    out.push_str(&format!("{}**{}:** {}\n", indent, label, value));
                }
            }
            let ingredients = entries("ingredients");
            if !ingredients.is_empty() {
                out.push_str(&format!("{}**Ingredients:**\n", indent));
                for ingredient in ingredients {
                    out.push_str(&format!("{}- {}\n", indent, ingredient));
                }
            }
            let steps = entries("steps");
            if !steps.is_empty() {
                out.push_str(&format!("{}**Steps:**\n", indent));
                for (index, step) in steps.iter().enumerate() {
                    out.push_str(&format!(
                        "{}{}. {}\n",
                        indent,
                        index.saturating_add(1),
                        step
                    ));
                }
            }
        }
        Primitive::Research => {
            let title = text("title");
            if !title.is_empty() {
                out.push_str(&format!("{}## {}\n", indent, title));
            }
            let question = text("question");
            if !question.is_empty() {
                out.push_str(&format!("{}**Question:** {}\n", indent, question));
            }
            for (label, key) in [("Takeaways", "takeaways"), ("Sources", "sources")] {
                let values = entries(key);
                if !values.is_empty() {
                    out.push_str(&format!("{}**{}:**\n", indent, label));
                    for value in values {
                        out.push_str(&format!("{}- {}\n", indent, value));
                    }
                }
            }
        }
        Primitive::UiPreview => {
            // The previewed document is untrusted authored markup; plain-text
            // surfaces describe it rather than inlining it.
            let title = text("title");
            if !title.is_empty() {
                out.push_str(&format!("{}## {}\n", indent, title));
            }
            let caption = text("caption");
            if !caption.is_empty() {
                out.push_str(&format!("{}{}\n", indent, caption));
            }
            out.push_str(&format!(
                "{}_(interactive preview omitted on this surface)_\n",
                indent
            ));
        }
    }
    for child in &node.children {
        out.push_str(&render_node(child, depth.saturating_add(1)));
    }
    out
}

/// Lower a list-shaped prop to plain lines. Stays structural: a string item is
/// used as-is, an object item is reduced through the same generic label/value
/// keys the rest of this lowering uses, never through domain field names.
fn entry_lines(node: &RenderNode, key: &str) -> Vec<String> {
    let Some(items) = node.props.get(key).and_then(|value| value.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            Value::String(value) => Some(value.clone()),
            Value::Object(fields) => {
                let pick = |name: &str| fields.get(name).and_then(|v| v.as_str()).unwrap_or("");
                let label = [pick("label"), pick("name"), pick("title")]
                    .into_iter()
                    .find(|value| !value.is_empty())
                    .unwrap_or_default();
                let value = [pick("text"), pick("value"), pick("detail")]
                    .into_iter()
                    .find(|value| !value.is_empty())
                    .unwrap_or_default();
                match (label.is_empty(), value.is_empty()) {
                    (true, true) => None,
                    (true, false) => Some(value.to_owned()),
                    (false, true) => Some(label.to_owned()),
                    (false, false) => Some(format!("{label}: {value}")),
                }
            }
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
        .collect()
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
            "root": { "primitive": "title", "props": { "text": { "kind": "binding", "path": "$.title" } }, "children": [] }
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
