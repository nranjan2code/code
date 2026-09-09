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
        | Primitive::Stale => {
            let value = text("text");
            if !value.is_empty() {
                out.push_str(&format!("{}{}\n", indent, value));
            }
        }
    }
    for child in &node.children {
        out.push_str(&render_node(child, depth.saturating_add(1)));
    }
    out
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
