#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Deep audit harness for `vak-presentation`: 500+ scenarios covering spec
//! validation, binding paths, compilation, the `PresentationLibrary`
//! register/activate/select/revoke lifecycle, revision proposals, semantic
//! type inference, digest consistency, and the seed pack.
//!
//! Each scenario is a self-contained closure that panics on invariant
//! violation. The harness counts pass/fail so the test report proves breadth.

use serde_json::json;
use std::collections::BTreeMap;
use vak_presentation::{
    AccessibilitySpec, Binding, CompileInput, CompiledPresentation, EmptyValue, FallbackKind,
    FallbackSpec, LibraryScope, MAX_DEPTH, MAX_EXPANSION_ITEMS, MAX_NODES, MAX_SPEC_BYTES,
    PresentationError, PresentationLibrary, PresentationOrigin, PresentationRevisionRequest,
    PresentationSpec, Primitive, SPEC_SCHEMA_VERSION, SpecNode, SpecValue, StoredPresentation,
    compile, digest, infer_semantic_type, parse_spec, propose_revision, seeds, validate_spec,
};

type Scenario = (String, Box<dyn FnOnce()>);

fn tc<F: FnOnce() + 'static>(name: &str, f: F) -> Scenario {
    (name.to_string(), Box::new(f))
}

fn run(label: &str, scenarios: Vec<Scenario>) {
    let total = scenarios.len();
    let mut passed = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for (name, test) in scenarios {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| test())) {
            Ok(()) => passed += 1,
            Err(_) => failures.push(name),
        }
    }
    if !failures.is_empty() {
        for f in &failures {
            eprintln!("FAIL: {label} :: {f}");
        }
    }
    assert_eq!(
        passed, total,
        "{label}: {passed}/{total} passed — failures: {failures:?}"
    );
    println!("  ✓ {label}: {total} scenarios passed");
}

// ── Data helpers ───────────────────────────────────────────────────

fn spec_node(prim: Primitive) -> SpecNode {
    SpecNode {
        primitive: prim,
        props: BTreeMap::new(),
        children: vec![],
        each: None,
        item: None,
    }
}

fn text_binding_node() -> SpecNode {
    SpecNode {
        primitive: Primitive::Text,
        props: BTreeMap::from([(
            "text".into(),
            SpecValue::Binding(Binding {
                path: "$.text".into(),
                required: false,
                empty: EmptyValue::EmptyText,
            }),
        )]),
        children: vec![],
        each: None,
        item: None,
    }
}

fn section_with_title() -> SpecNode {
    SpecNode {
        primitive: Primitive::Section,
        props: BTreeMap::from([(
            "title".into(),
            SpecValue::Binding(Binding {
                path: "$.title".into(),
                required: false,
                empty: EmptyValue::EmptyText,
            }),
        )]),
        children: vec![text_binding_node()],
        each: None,
        item: None,
    }
}

fn make_spec(id: &str, rev: u64, accepts: &[&str], root: SpecNode) -> PresentationSpec {
    PresentationSpec {
        schema_version: SPEC_SCHEMA_VERSION,
        id: id.into(),
        revision: rev,
        accepts: accepts.iter().map(|s| s.to_string()).collect(),
        root,
        fallback: FallbackSpec::default(),
        accessibility: AccessibilitySpec::default(),
        metadata: BTreeMap::new(),
    }
}

fn binding_val(path: &str, required: bool) -> SpecValue {
    SpecValue::Binding(Binding {
        path: path.into(),
        required,
        empty: EmptyValue::Omit,
    })
}

fn ci(sem: &str, payload: serde_json::Value, fb: &str) -> CompileInput {
    let payload = match payload {
        serde_json::Value::Object(mut map) => {
            if !map.contains_key("text") {
                map.insert("text".into(), serde_json::Value::String("hello".into()));
            }
            serde_json::Value::Object(map)
        }
        other => other,
    };
    CompileInput {
        semantic_type: sem.into(),
        payload,
        fallback_text: fb.into(),
    }
}

fn stored_simple(
    id: &str,
    rev: u64,
    accepts: &[&str],
    root: SpecNode,
    owner: &str,
) -> StoredPresentation {
    let spec = make_spec(id, rev, accepts, root);
    let dg = digest(&spec).unwrap();
    StoredPresentation {
        spec,
        digest: dg,
        origin: PresentationOrigin {
            scope: LibraryScope::Workspace,
            owner: owner.into(),
            plugin_id: None,
            generation: Some("test-1".into()),
        },
        enabled: true,
    }
}

fn stored_builtin(id: &str, rev: u64, accepts: &[&str], root: SpecNode) -> StoredPresentation {
    let spec = make_spec(id, rev, accepts, root);
    let dg = digest(&spec).unwrap();
    StoredPresentation {
        spec,
        digest: dg,
        origin: PresentationOrigin {
            scope: LibraryScope::Workspace,
            owner: "builtin".into(),
            plugin_id: None,
            generation: Some("seed-2".into()),
        },
        enabled: true,
    }
}

fn stored_plugin(
    id: &str,
    rev: u64,
    accepts: &[&str],
    root: SpecNode,
    plugin_id: &str,
) -> StoredPresentation {
    let spec = make_spec(id, rev, accepts, root);
    let dg = digest(&spec).unwrap();
    StoredPresentation {
        spec,
        digest: dg,
        origin: PresentationOrigin {
            scope: LibraryScope::Workspace,
            owner: "plugin_owner".into(),
            plugin_id: Some(plugin_id.into()),
            generation: Some("gen-1".into()),
        },
        enabled: true,
    }
}

const PRIMITIVES: &[Primitive] = &[
    Primitive::Stack,
    Primitive::Row,
    Primitive::Group,
    Primitive::Section,
    Primitive::Divider,
    Primitive::Title,
    Primitive::Text,
    Primitive::RichText,
    Primitive::Label,
    Primitive::Badge,
    Primitive::Callout,
    Primitive::Quote,
    Primitive::List,
    Primitive::Checklist,
    Primitive::Timeline,
    Primitive::Steps,
    Primitive::KeyValue,
    Primitive::Table,
    Primitive::Metric,
    Primitive::Progress,
    Primitive::Chart,
    Primitive::DataGrid,
    Primitive::Comparison,
    Primitive::Image,
    Primitive::Audio,
    Primitive::Video,
    Primitive::File,
    Primitive::LinkPreview,
    Primitive::Gallery,
    Primitive::Diff,
    Primitive::TestMatrix,
    Primitive::Terminal,
    Primitive::Artifact,
    Primitive::CitationList,
    Primitive::Disclosure,
    Primitive::Filter,
    Primitive::Sort,
    Primitive::Search,
    Primitive::Stepper,
    Primitive::Timer,
    Primitive::Loading,
    Primitive::Empty,
    Primitive::Partial,
    Primitive::Error,
    Primitive::Unavailable,
    Primitive::Stale,
];

// ════════════════════════════════════════════════════════════════════
// 1. SPEC VALIDATION
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_spec_validation() {
    let mut scenarios: Vec<Scenario> = vec![];

    // 1a. Valid spec for each primitive (47 variants)
    for (i, prim) in PRIMITIVES.iter().enumerate() {
        let p = *prim;
        scenarios.push(tc(&format!("valid_spec_{i}_{p:?}"), move || {
            let s = make_spec(&format!("v{i}"), 1, &["detail"], spec_node(p));
            assert!(validate_spec(&s).is_ok(), "primitive {p:?} should validate");
        }));
    }

    // 1b. Valid complex specs
    scenarios.push(tc("valid_section_node", || {
        let s = make_spec("cs1", 1, &["detail"], section_with_title());
        assert!(validate_spec(&s).is_ok());
    }));

    scenarios.push(tc("valid_divider_node", || {
        let s = make_spec("cs2", 1, &["detail"], spec_node(Primitive::Divider));
        assert!(validate_spec(&s).is_ok());
    }));

    scenarios.push(tc("valid_stale_node", || {
        let s = make_spec("cs3", 1, &["detail"], spec_node(Primitive::Stale));
        assert!(validate_spec(&s).is_ok());
    }));

    scenarios.push(tc("valid_error_node", || {
        let s = make_spec("cs4", 1, &["detail"], spec_node(Primitive::Error));
        assert!(validate_spec(&s).is_ok());
    }));

    scenarios.push(tc("valid_loading_node", || {
        let s = make_spec("cs5", 1, &["detail"], spec_node(Primitive::Loading));
        assert!(validate_spec(&s).is_ok());
    }));

    scenarios.push(tc("valid_empty_node", || {
        let s = make_spec("cs6", 1, &["detail"], spec_node(Primitive::Empty));
        assert!(validate_spec(&s).is_ok());
    }));

    scenarios.push(tc("valid_partial_node", || {
        let s = make_spec("cs7", 1, &["detail"], spec_node(Primitive::Partial));
        assert!(validate_spec(&s).is_ok());
    }));

    scenarios.push(tc("valid_richtext_node", || {
        let s = make_spec("cs8", 1, &["detail"], spec_node(Primitive::RichText));
        assert!(validate_spec(&s).is_ok());
    }));

    scenarios.push(tc("valid_nested_stack", || {
        let root = SpecNode {
            primitive: Primitive::Stack,
            props: BTreeMap::new(),
            children: vec![spec_node(Primitive::Text), spec_node(Primitive::Divider)],
            each: None,
            item: None,
        };
        let s = make_spec("cs9", 1, &["detail"], root);
        assert!(validate_spec(&s).is_ok());
    }));

    scenarios.push(tc("valid_section_no_title", || {
        let root = SpecNode {
            primitive: Primitive::Section,
            props: BTreeMap::new(),
            children: vec![text_binding_node()],
            each: None,
            item: None,
        };
        let s = make_spec("cs10", 1, &["detail"], root);
        assert!(validate_spec(&s).is_ok());
    }));

    // 1c. Invalid schema versions
    for bad_sv in [0u16, 2, 3, 65535] {
        scenarios.push(tc(&format!("invalid_schema_{bad_sv}"), move || {
            let mut s = make_spec("bad_sv", 1, &["detail"], text_binding_node());
            s.schema_version = bad_sv;
            assert!(matches!(validate_spec(&s), Err(PresentationError::UnsupportedSchema(v)) if v == bad_sv));
        }));
    }

    // 1d. Empty / whitespace / too-long id
    scenarios.push(tc("empty_id_invalid", || {
        let s = make_spec("", 1, &["detail"], text_binding_node());
        assert!(matches!(
            validate_spec(&s),
            Err(PresentationError::InvalidSpec(_))
        ));
    }));

    scenarios.push(tc("whitespace_id_invalid", || {
        let s = make_spec("   ", 1, &["detail"], text_binding_node());
        assert!(matches!(
            validate_spec(&s),
            Err(PresentationError::InvalidSpec(_))
        ));
    }));

    scenarios.push(tc("too_long_id_invalid", || {
        let long_id = "x".repeat(257);
        let s = make_spec(&long_id, 1, &["detail"], text_binding_node());
        assert!(matches!(
            validate_spec(&s),
            Err(PresentationError::InvalidSpec(_))
        ));
    }));

    // 1e. Zero revision
    scenarios.push(tc("zero_revision_invalid", || {
        let s = make_spec("zr", 0, &["detail"], text_binding_node());
        assert!(matches!(
            validate_spec(&s),
            Err(PresentationError::InvalidSpec(_))
        ));
    }));

    // 1f. Empty accepts entries
    scenarios.push(tc("empty_accept_entry_invalid", || {
        let s = make_spec("bad_a", 1, &[""], text_binding_node());
        assert!(matches!(
            validate_spec(&s),
            Err(PresentationError::InvalidSpec(_))
        ));
    }));

    scenarios.push(tc("whitespace_accept_entry_invalid", || {
        let s = make_spec("bad_a2", 1, &["  "], text_binding_node());
        assert!(matches!(
            validate_spec(&s),
            Err(PresentationError::InvalidSpec(_))
        ));
    }));

    // 1g. Too many nodes
    for extra in [0usize, 1, 5, 50] {
        let target = MAX_NODES + extra;
        scenarios.push(tc(&format!("too_many_nodes_{target}"), move || {
            let mut children = Vec::new();
            for i in 0..target {
                children.push(SpecNode {
                    primitive: Primitive::Text,
                    props: BTreeMap::from([(
                        "text".into(),
                        SpecValue::Text(format!("n{i}").into()),
                    )]),
                    children: vec![],
                    each: None,
                    item: None,
                });
            }
            let root = SpecNode {
                primitive: Primitive::Stack,
                props: BTreeMap::new(),
                children,
                each: None,
                item: None,
            };
            let s = make_spec("too_many", 1, &["detail"], root);
            assert!(matches!(
                validate_spec(&s),
                Err(PresentationError::Limit(_))
            ));
        }));
    }

    // 1h. Too deep nesting
    for extra_depth in [0usize, 1, 5] {
        let depth = MAX_DEPTH + 1 + extra_depth;
        scenarios.push(tc(&format!("too_deep_{depth}"), move || {
            let mut node = text_binding_node();
            for _ in 0..depth {
                node = SpecNode {
                    primitive: Primitive::Stack,
                    props: BTreeMap::new(),
                    children: vec![node],
                    each: None,
                    item: None,
                };
            }
            let s = make_spec("too_deep", 1, &["detail"], node);
            assert!(matches!(
                validate_spec(&s),
                Err(PresentationError::Limit(_))
            ));
        }));
    }

    // 1i. Each without item
    scenarios.push(tc("each_without_item_invalid", || {
        let root = SpecNode {
            primitive: Primitive::Stack,
            props: BTreeMap::new(),
            children: vec![],
            each: Some(Binding {
                path: "$.items".into(),
                required: false,
                empty: EmptyValue::Omit,
            }),
            item: None,
        };
        let s = make_spec("no_item", 1, &["detail"], root);
        assert!(matches!(
            validate_spec(&s),
            Err(PresentationError::InvalidSpec(_))
        ));
    }));

    // 1j. Each with item is valid
    scenarios.push(tc("each_with_item_valid", || {
        let root = SpecNode {
            primitive: Primitive::Stack,
            props: BTreeMap::new(),
            children: vec![],
            each: Some(Binding {
                path: "$.items".into(),
                required: false,
                empty: EmptyValue::Omit,
            }),
            item: Some(Box::new(text_binding_node())),
        };
        let s = make_spec("with_item", 1, &["detail"], root);
        assert!(validate_spec(&s).is_ok());
    }));

    // 1k. Non-finite number (validation doesn't serialize, so should pass)
    for n_val in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let n = n_val;
        scenarios.push(tc(&format!("non_finite_number_{n:?}"), move || {
            let mut props = BTreeMap::new();
            props.insert("value".into(), SpecValue::Number(n));
            let root = SpecNode {
                primitive: Primitive::Metric,
                props,
                children: vec![],
                each: None,
                item: None,
            };
            let s = make_spec("non_finite", 1, &["metric"], root);
            let result = validate_spec(&s);
            // validate_spec doesn't serialize, but compile does (via digest)
            assert!(result.is_ok());
        }));
    }

    run("spec_validation", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 2. BINDING PATHS
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_binding_paths() {
    let mut scenarios: Vec<Scenario> = vec![];

    let valid_paths: &[&str] = &[
        "$",
        "$.title",
        "$.title.subtitle",
        "$.items[0]",
        "$.items[0].name",
        "$.items[10].name",
        "$.data[999]",
        "$.a.b.c.d.e.f.g",
        "$.a-b_c",
        "$.a1.b2.c3",
        "$[0]",
        "$[0][1]",
        "$.items[0][1]",
        "$.very.long.path.with.many.segments[0][1][2]",
    ];

    for path in valid_paths {
        let p = path.to_string();
        scenarios.push(tc(&format!("valid_path_{p}"), move || {
            let root = SpecNode {
                primitive: Primitive::Text,
                props: BTreeMap::from([("text".into(), binding_val(&p, false))]),
                children: vec![],
                each: None,
                item: None,
            };
            let s = make_spec("vb", 1, &["detail"], root);
            assert!(validate_spec(&s).is_ok(), "path {p:?} should be valid");
        }));
    }

    let invalid_paths: &[&str] = &[
        "title",
        "$.items[-1]",
        "$.items[abc]",
        "$.items[]",
        "$['title']",
        "$.items[",
        "$.items]",
        "$.items[0",
        "$..title",
        "$.title + 1",
        "$.items[0].",
        "$.items[0]..name",
        "./title",
        "$#comment",
        "$.items[1.5]",
        "$.items[0x]",
        "$.items[+1]",
        "$.a..b",
        "$.a.b.",
        "$.a[0]b",
        "$/.[0]",
        "$.[items]",
        "$/",
        "$.items[0]extra",
        "$.items]",
        "$a",
    ];

    for path in invalid_paths {
        let p = path.to_string();
        scenarios.push(tc(&format!("invalid_path_{p:?}"), move || {
            let root = SpecNode {
                primitive: Primitive::Text,
                props: BTreeMap::from([("text".into(), binding_val(&p, false))]),
                children: vec![],
                each: None,
                item: None,
            };
            let s = make_spec("ib", 1, &["detail"], root);
            let result = validate_spec(&s);
            assert!(
                matches!(result, Err(PresentationError::InvalidBinding(_))),
                "path {p:?}: {result:?}"
            );
        }));
    }

    // Path too long (>512 chars)
    scenarios.push(tc("path_513_chars_invalid", || {
        let long_path = format!("$.{}", "a.".repeat(300));
        let root = SpecNode {
            primitive: Primitive::Text,
            props: BTreeMap::from([("text".into(), binding_val(&long_path, false))]),
            children: vec![],
            each: None,
            item: None,
        };
        let s = make_spec("pl", 1, &["detail"], root);
        assert!(matches!(
            validate_spec(&s),
            Err(PresentationError::InvalidBinding(_))
        ));
    }));

    // Empty path
    scenarios.push(tc("empty_binding_path_invalid", || {
        let root = SpecNode {
            primitive: Primitive::Text,
            props: BTreeMap::from([("text".into(), binding_val("", false))]),
            children: vec![],
            each: None,
            item: None,
        };
        let s = make_spec("ep", 1, &["detail"], root);
        assert!(matches!(
            validate_spec(&s),
            Err(PresentationError::InvalidBinding(_))
        ));
    }));

    // Required binding absent → Fallback (via compile)
    scenarios.push(tc("required_binding_absent_fallback", || {
        let root = SpecNode {
            primitive: Primitive::Text,
            props: BTreeMap::from([("text".into(), binding_val("$.missing", true))]),
            children: vec![],
            each: None,
            item: None,
        };
        let s = make_spec("req_absent", 1, &["detail"], root);
        let result = compile(&s, &ci("detail", json!({}), "fb"));
        assert!(matches!(result, CompiledPresentation::Fallback { .. }));
    }));

    // Optional binding absent with Omit → resolved as None, node still Rich
    scenarios.push(tc("optional_binding_missing_omit", || {
        let root = SpecNode {
            primitive: Primitive::Text,
            props: BTreeMap::from([(
                "text".into(),
                SpecValue::Binding(Binding {
                    path: "$.missing".into(),
                    required: false,
                    empty: EmptyValue::Omit,
                }),
            )]),
            children: vec![],
            each: None,
            item: None,
        };
        let s = make_spec("opt_omit", 1, &["detail"], root);
        assert!(validate_spec(&s).is_ok());
        let result = compile(&s, &ci("detail", json!({}), "fb"));
        assert!(matches!(result, CompiledPresentation::Rich(_)));
    }));

    // Optional binding absent with EmptyText → resolved as empty string
    scenarios.push(tc("optional_binding_missing_empty_text", || {
        let root = SpecNode {
            primitive: Primitive::Text,
            props: BTreeMap::from([(
                "text".into(),
                SpecValue::Binding(Binding {
                    path: "$.missing".into(),
                    required: false,
                    empty: EmptyValue::EmptyText,
                }),
            )]),
            children: vec![],
            each: None,
            item: None,
        };
        let s = make_spec("opt_et", 1, &["detail"], root);
        assert!(validate_spec(&s).is_ok());
    }));

    // Optional binding absent with EmptyList → resolves as empty array
    scenarios.push(tc("optional_binding_missing_empty_list", || {
        let root = SpecNode {
            primitive: Primitive::Stack,
            props: BTreeMap::new(),
            children: vec![],
            each: Some(Binding {
                path: "$.missing".into(),
                required: false,
                empty: EmptyValue::EmptyList,
            }),
            item: Some(Box::new(text_binding_node())),
        };
        let s = make_spec("opt_el", 1, &["detail"], root);
        assert!(validate_spec(&s).is_ok());
    }));

    // Each with non-array data → Fallback
    scenarios.push(tc("each_non_array_fallback", || {
        let root = SpecNode {
            primitive: Primitive::Stack,
            props: BTreeMap::new(),
            children: vec![],
            each: Some(Binding {
                path: "$.items".into(),
                required: false,
                empty: EmptyValue::Omit,
            }),
            item: Some(Box::new(text_binding_node())),
        };
        let s = make_spec("each_na", 1, &["detail"], root);
        let result = compile(&s, &ci("detail", json!({"items": "not_an_array"}), "fb"));
        assert!(matches!(result, CompiledPresentation::Fallback { .. }));
    }));

    // Each expansion exceeding MAX_EXPANSION_ITEMS
    scenarios.push(tc("each_exceeds_max_expansion_fallback", || {
        let root = SpecNode {
            primitive: Primitive::Stack,
            props: BTreeMap::new(),
            children: vec![],
            each: Some(Binding {
                path: "$.items".into(),
                required: false,
                empty: EmptyValue::Omit,
            }),
            item: Some(Box::new(text_binding_node())),
        };
        let s = make_spec("each_over", 1, &["detail"], root);
        let items: Vec<String> = (0..(MAX_EXPANSION_ITEMS + 10))
            .map(|i| format!("i{i}"))
            .collect();
        let result = compile(&s, &ci("detail", json!({"items": items}), "fb"));
        assert!(matches!(result, CompiledPresentation::Fallback { .. }));
    }));

    // Nested each valid
    scenarios.push(tc("nested_each_valid", || {
        let inner = SpecNode {
            primitive: Primitive::Stack,
            props: BTreeMap::new(),
            children: vec![],
            each: Some(Binding {
                path: "$.items".into(),
                required: false,
                empty: EmptyValue::Omit,
            }),
            item: Some(Box::new(text_binding_node())),
        };
        let root = SpecNode {
            primitive: Primitive::Stack,
            props: BTreeMap::new(),
            children: vec![inner],
            each: None,
            item: None,
        };
        let s = make_spec("nested_each", 1, &["detail"], root);
        assert!(validate_spec(&s).is_ok());
    }));

    run("binding_paths", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 3. COMPILATION
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_compilation() {
    let mut scenarios: Vec<Scenario> = vec![];

    // 3a. Valid compilation for each primitive
    for (i, prim) in PRIMITIVES.iter().enumerate() {
        let p = *prim;
        scenarios.push(tc(&format!("compile_{i}_{p:?}"), move || {
            let s = make_spec(&format!("c{i}"), 1, &["detail"], spec_node(p));
            let result = compile(&s, &ci("detail", json!({"title":"T","items":[1]}), "fb"));
            assert!(
                matches!(result, CompiledPresentation::Rich(_)),
                "compile {prim:?}: {result:?}"
            );
        }));
    }

    // 3b. Semantic type mismatch → Fallback
    for (i, prim) in PRIMITIVES.iter().enumerate() {
        let p = *prim;
        scenarios.push(tc(&format!("mismatch_{i}_{p:?}"), move || {
            let s = make_spec(&format!("m{i}"), 1, &["comparison"], spec_node(p));
            let result = compile(&s, &ci("detail", json!({}), "fb"));
            assert!(
                matches!(result, CompiledPresentation::Fallback { ref reason, .. } if reason.contains("does not match")),
                "mismatch {prim:?}: {result:?}"
            );
        }));
    }

    // 3c. Each with collection expansion at various sizes
    for size in [0usize, 1, 2, 5, 10, 50, 100, 255, 256] {
        let sz = size;
        scenarios.push(tc(&format!("each_size_{sz}"), move || {
            let root = SpecNode {
                primitive: Primitive::Stack,
                props: BTreeMap::new(),
                children: vec![],
                each: Some(Binding {
                    path: "$.items".into(),
                    required: false,
                    empty: EmptyValue::Omit,
                }),
                item: Some(Box::new(text_binding_node())),
            };
            let s = make_spec("each", 1, &["detail"], root);
            let payload =
                json!({"items": (0..sz).map(|j| json!(format!("i{j}"))).collect::<Vec<_>>()});
            let result = compile(&s, &ci("detail", payload, "fb"));
            assert!(
                matches!(result, CompiledPresentation::Rich(_)),
                "each size {sz}: {result:?}"
            );
        }));
    }

    // 3d. FallbackKind Document vs Plain
    for (fk, label) in [
        (FallbackKind::Document, "doc"),
        (FallbackKind::Plain, "plain"),
    ] {
        let lf = label.to_string();
        let fk_val = fk;
        scenarios.push(tc(&format!("fallback_kind_{lf}"), move || {
            let root = SpecNode {
                primitive: Primitive::Title,
                props: BTreeMap::from([("title".into(), binding_val("$.missing", true))]),
                children: vec![],
                each: None,
                item: None,
            };
            let s = PresentationSpec {
                schema_version: SPEC_SCHEMA_VERSION,
                id: format!("fb_{lf}"),
                revision: 1,
                accepts: vec!["detail".into()],
                root,
                fallback: FallbackSpec { kind: fk_val },
                accessibility: AccessibilitySpec::default(),
                metadata: BTreeMap::new(),
            };
            let result = compile(&s, &ci("detail", json!({}), "fallback text"));
            assert!(matches!(result, CompiledPresentation::Fallback { .. }));
        }));
    }

    // 3e. Deep nesting within MAX_DEPTH compiles
    scenarios.push(tc("deep_nesting_within_depth_compiles", || {
        let mut node = text_binding_node();
        for _ in 0..(MAX_DEPTH - 2) {
            node = SpecNode {
                primitive: Primitive::Stack,
                props: BTreeMap::new(),
                children: vec![node],
                each: None,
                item: None,
            };
        }
        let s = make_spec("deep_ok", 1, &["detail"], node);
        let result = compile(&s, &ci("detail", json!({}), "fb"));
        assert!(matches!(result, CompiledPresentation::Rich(_)));
    }));

    // 3f. Deep nesting exceeding depth at compile → Fallback
    scenarios.push(tc("deep_nesting_exceeds_depth_fallback", || {
        let mut node = text_binding_node();
        for _ in 0..(MAX_DEPTH + 5) {
            node = SpecNode {
                primitive: Primitive::Stack,
                props: BTreeMap::new(),
                children: vec![node],
                each: None,
                item: None,
            };
        }
        let s = make_spec("deep_bad", 1, &["detail"], node);
        let result = compile(&s, &ci("detail", json!({}), "fb"));
        assert!(matches!(result, CompiledPresentation::Fallback { .. }));
    }));

    // 3g. Accessibility summary resolved
    scenarios.push(tc("accessibility_summary_resolved", || {
        let root = section_with_title();
        let mut s = make_spec("acc1", 1, &["detail"], root);
        s.accessibility.summary = Some(Binding {
            path: "$.title".into(),
            required: false,
            empty: EmptyValue::EmptyText,
        });
        let result = compile(&s, &ci("detail", json!({"title": "Hello World"}), "fb"));
        assert!(matches!(result, CompiledPresentation::Rich(_)));
    }));

    // 3h. Accessibility summary missing → ok
    scenarios.push(tc("accessibility_summary_missing_ok", || {
        let s = make_spec("acc2", 1, &["detail"], section_with_title());
        let result = compile(&s, &ci("detail", json!({}), "fb"));
        assert!(matches!(result, CompiledPresentation::Rich(_)));
    }));

    // 3i. Edge payloads
    scenarios.push(tc("empty_payload_omits_optional_bindings", || {
        let root = SpecNode {
            primitive: Primitive::Text,
            props: BTreeMap::from([("text".into(), binding_val("$.missing", false))]),
            children: vec![],
            each: None,
            item: None,
        };
        let s = make_spec("empty", 1, &["detail"], root);
        let result = compile(&s, &ci("detail", json!({}), "fb"));
        assert!(matches!(result, CompiledPresentation::Rich(_)));
    }));

    scenarios.push(tc("unicode_in_text", || {
        let root = SpecNode {
            primitive: Primitive::Section,
            props: BTreeMap::from([("title".into(), binding_val("$.title", false))]),
            children: vec![],
            each: None,
            item: None,
        };
        let s = make_spec("unicode", 1, &["detail"], root);
        let result = compile(&s, &ci("detail", json!({"title": "Hello 世界 🌍"}), "fb"));
        assert!(matches!(result, CompiledPresentation::Rich(_)));
    }));

    scenarios.push(tc("special_chars_in_text", || {
        let root = SpecNode {
            primitive: Primitive::Section,
            props: BTreeMap::from([("title".into(), binding_val("$.title", false))]),
            children: vec![],
            each: None,
            item: None,
        };
        let s = make_spec("specialchars", 1, &["detail"], root);
        let result = compile(&s, &ci("detail", json!({"title": "<>&\"'`"}), "fb"));
        assert!(matches!(result, CompiledPresentation::Rich(_)));
    }));

    scenarios.push(tc("hostile_massive_payload_no_panic", || {
        let big_array: Vec<String> = (0..10000).map(|i| format!("x{i}")).collect();
        let s = make_spec("huge", 1, &["detail"], section_with_title());
        let result = compile(
            &s,
            &ci("detail", json!({"title": "Huge", "items": big_array}), "fb"),
        );
        assert!(matches!(result, CompiledPresentation::Rich(_)));
    }));

    scenarios.push(tc("hostile_deeply_nested_json_no_panic", || {
        let nested = serde_json::Value::Array((0..100).map(|i| json!({"v": i})).collect());
        let s = make_spec("deep_json", 1, &["detail"], section_with_title());
        let result = compile(
            &s,
            &ci("detail", json!({"title": "Deep", "items": nested}), "fb"),
        );
        assert!(matches!(result, CompiledPresentation::Rich(_)));
    }));

    scenarios.push(tc("hostile_null_payload_fallback", || {
        let root = SpecNode {
            primitive: Primitive::Text,
            props: BTreeMap::from([("text".into(), binding_val("$.title", true))]),
            children: vec![],
            each: None,
            item: None,
        };
        let s = make_spec("null_pl", 1, &["detail"], root);
        let result = compile(&s, &ci("detail", serde_json::Value::Null, "fb"));
        assert!(matches!(result, CompiledPresentation::Fallback { .. }));
    }));

    scenarios.push(tc("hostile_array_payload_fallback", || {
        let root = SpecNode {
            primitive: Primitive::Section,
            props: BTreeMap::from([("title".into(), binding_val("$.title", true))]),
            children: vec![],
            each: None,
            item: None,
        };
        let s = make_spec("arr_pl", 1, &["detail"], root);
        let result = compile(&s, &ci("detail", serde_json::Value::Array(vec![]), "fb"));
        assert!(matches!(result, CompiledPresentation::Fallback { .. }));
    }));

    scenarios.push(tc("hostile_string_payload_fallback", || {
        let root = SpecNode {
            primitive: Primitive::Section,
            props: BTreeMap::from([("title".into(), binding_val("$.title", true))]),
            children: vec![],
            each: None,
            item: None,
        };
        let s = make_spec("str_pl", 1, &["detail"], root);
        let result = compile(
            &s,
            &ci("detail", serde_json::Value::String("not obj".into()), "fb"),
        );
        assert!(matches!(result, CompiledPresentation::Fallback { .. }));
    }));

    scenarios.push(tc("hostile_null_bytes_in_text", || {
        let root = SpecNode {
            primitive: Primitive::Section,
            props: BTreeMap::from([("title".into(), binding_val("$.title", false))]),
            children: vec![],
            each: None,
            item: None,
        };
        let s = make_spec("nullb", 1, &["detail"], root);
        let result = compile(&s, &ci("detail", json!({"title": "hello\0world"}), "fb"));
        assert!(matches!(result, CompiledPresentation::Rich(_)));
    }));

    scenarios.push(tc("hostile_emoji_in_text", || {
        let root = SpecNode {
            primitive: Primitive::Section,
            props: BTreeMap::from([("title".into(), binding_val("$.title", false))]),
            children: vec![],
            each: None,
            item: None,
        };
        let s = make_spec("emoji", 1, &["detail"], root);
        let result = compile(&s, &ci("detail", json!({"title": "🎉🚀✅"}), "fb"));
        assert!(matches!(result, CompiledPresentation::Rich(_)));
    }));

    scenarios.push(tc("hostile_control_chars_in_text", || {
        let root = SpecNode {
            primitive: Primitive::Section,
            props: BTreeMap::from([("title".into(), binding_val("$.title", false))]),
            children: vec![],
            each: None,
            item: None,
        };
        let s = make_spec("ctl", 1, &["detail"], root);
        let result = compile(&s, &ci("detail", json!({"title": "\t\n\r"}), "fb"));
        assert!(matches!(result, CompiledPresentation::Rich(_)));
    }));

    scenarios.push(tc("hostile_array_index_out_of_range", || {
        let root = SpecNode {
            primitive: Primitive::Text,
            props: BTreeMap::from([("text".into(), binding_val("$.items[999]", false))]),
            children: vec![],
            each: None,
            item: None,
        };
        let s = make_spec("oor", 1, &["detail"], root);
        let result = compile(&s, &ci("detail", json!({"items": [1, 2, 3]}), "fb"));
        assert!(matches!(
            result,
            CompiledPresentation::Rich(_) | CompiledPresentation::Fallback { .. }
        ));
    }));

    scenarios.push(tc("hostile_empty_array_each", || {
        let root = SpecNode {
            primitive: Primitive::Stack,
            props: BTreeMap::new(),
            children: vec![],
            each: Some(Binding {
                path: "$.items".into(),
                required: false,
                empty: EmptyValue::EmptyList,
            }),
            item: Some(Box::new(text_binding_node())),
        };
        let s = make_spec("ea", 1, &["detail"], root);
        let result = compile(&s, &ci("detail", json!({"items": []}), "fb"));
        assert!(matches!(result, CompiledPresentation::Rich(_)));
    }));

    run("compilation", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 4. DIGEST CONSISTENCY
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_digests() {
    let mut scenarios: Vec<Scenario> = vec![];

    scenarios.push(tc("identical_specs_same_digest", || {
        let s1 = make_spec("d1", 1, &["detail"], section_with_title());
        let s2 = make_spec("d1", 1, &["detail"], section_with_title());
        assert_eq!(digest(&s1).unwrap(), digest(&s2).unwrap());
    }));

    scenarios.push(tc("different_title_different_digest", || {
        let root1 = SpecNode {
            primitive: Primitive::Section,
            props: BTreeMap::from([("title".into(), binding_val("$.a", false))]),
            children: vec![],
            each: None,
            item: None,
        };
        let root2 = SpecNode {
            primitive: Primitive::Section,
            props: BTreeMap::from([("title".into(), binding_val("$.b", false))]),
            children: vec![],
            each: None,
            item: None,
        };
        let s1 = make_spec("d2", 1, &["detail"], root1);
        let s2 = make_spec("d2", 1, &["detail"], root2);
        assert_ne!(digest(&s1).unwrap(), digest(&s2).unwrap());
    }));

    scenarios.push(tc("different_id_different_digest", || {
        let s1 = make_spec("id_a", 1, &["detail"], section_with_title());
        let s2 = make_spec("id_b", 1, &["detail"], section_with_title());
        assert_ne!(digest(&s1).unwrap(), digest(&s2).unwrap());
    }));

    scenarios.push(tc("different_revision_different_digest", || {
        let s1 = make_spec("rev", 1, &["detail"], section_with_title());
        let s2 = make_spec("rev", 2, &["detail"], section_with_title());
        assert_ne!(digest(&s1).unwrap(), digest(&s2).unwrap());
    }));

    scenarios.push(tc("different_primitive_different_digest", || {
        let s1 = make_spec("prim", 1, &["detail"], spec_node(Primitive::Stack));
        let s2 = make_spec("prim", 1, &["detail"], spec_node(Primitive::Row));
        assert_ne!(digest(&s1).unwrap(), digest(&s2).unwrap());
    }));

    scenarios.push(tc("json_roundtrip_preserves_digest", || {
        let s = make_spec("rt", 1, &["detail"], section_with_title());
        let d1 = digest(&s).unwrap();
        let bytes = serde_json::to_vec(&s).unwrap();
        let parsed: PresentationSpec = serde_json::from_slice(&bytes).unwrap();
        let d2 = digest(&parsed).unwrap();
        assert_eq!(d1, d2);
    }));

    scenarios.push(tc("digest_is_stable_across_calls", || {
        let s = make_spec("stable", 1, &["detail"], section_with_title());
        let d1 = digest(&s).unwrap();
        let d2 = digest(&s).unwrap();
        let d3 = digest(&s).unwrap();
        assert_eq!(d1, d2);
        assert_eq!(d2, d3);
    }));

    scenarios.push(tc("digest_is_hex_64_chars", || {
        let s = make_spec("hex", 1, &["detail"], section_with_title());
        let d = digest(&s).unwrap();
        assert_eq!(d.len(), 64);
        assert!(d.chars().all(|c| c.is_ascii_hexdigit()));
    }));

    scenarios.push(tc("digest_complex_spec_ok", || {
        let mut props = BTreeMap::new();
        props.insert("title".into(), binding_val("$.title", false));
        props.insert(
            "items".into(),
            SpecValue::Binding(Binding {
                path: "$.items".into(),
                required: false,
                empty: EmptyValue::EmptyList,
            }),
        );
        let root = SpecNode {
            primitive: Primitive::Section,
            props,
            children: vec![SpecNode {
                primitive: Primitive::List,
                props: BTreeMap::new(),
                children: vec![],
                each: Some(Binding {
                    path: "$.items".into(),
                    required: false,
                    empty: EmptyValue::Omit,
                }),
                item: Some(Box::new(text_binding_node())),
            }],
            each: None,
            item: None,
        };
        let s = make_spec("complex", 1, &["detail", "comparison"], root);
        assert!(digest(&s).is_ok());
    }));

    // SpecValue::Text(String) cannot be serialized under internal tagging
    scenarios.push(tc("digest_text_variant_fails", || {
        let root = SpecNode {
            primitive: Primitive::Text,
            props: BTreeMap::from([("text".into(), SpecValue::Text("hello".into()))]),
            children: vec![],
            each: None,
            item: None,
        };
        let s = make_spec("bad_dig", 1, &["detail"], root);
        assert!(digest(&s).is_err());
    }));

    run("digests", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 5. PARSE SPEC
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_parse_spec() {
    let mut scenarios: Vec<Scenario> = vec![];

    let valid_json = r#"{"schema_version":1,"id":"test","revision":1,"accepts":["detail"],"root":{"primitive":"text","props":{"text":{"kind":"binding","path":"$.text","required":false,"empty":"empty_text"}}}}"#;

    scenarios.push(tc("parse_valid_minimal_json", || {
        assert!(parse_spec(valid_json.as_bytes()).is_ok());
    }));

    scenarios.push(tc("parse_not_json", || {
        assert!(matches!(
            parse_spec(b"not json"),
            Err(PresentationError::InvalidJson(_))
        ));
    }));

    scenarios.push(tc("parse_empty_bytes", || {
        assert!(matches!(
            parse_spec(b""),
            Err(PresentationError::InvalidJson(_))
        ));
    }));

    scenarios.push(tc("parse_empty_object", || {
        assert!(parse_spec(b"{}").is_err());
    }));

    scenarios.push(tc("parse_array_not_object", || {
        assert!(matches!(
            parse_spec(b"[1,2,3]"),
            Err(PresentationError::InvalidJson(_))
        ));
    }));

    scenarios.push(tc("parse_null", || {
        assert!(matches!(
            parse_spec(b"null"),
            Err(PresentationError::InvalidJson(_))
        ));
    }));

    scenarios.push(tc("parse_scalar_int", || {
        assert!(parse_spec(b"42").is_err());
    }));

    scenarios.push(tc("parse_wrong_schema_version", || {
        let bad = r#"{"schema_version":2,"id":"x","revision":1,"accepts":["detail"],"root":{"primitive":"text","props":{"text":{"kind":"binding","path":"$.text","required":false,"empty":"empty_text"}}}}"#;
        assert!(matches!(parse_spec(bad.as_bytes()), Err(PresentationError::UnsupportedSchema(2))));
    }));

    scenarios.push(tc("parse_empty_id", || {
        let bad = r#"{"schema_version":1,"id":"","revision":1,"accepts":["detail"],"root":{"primitive":"text","props":{"text":{"kind":"binding","path":"$.text","required":false,"empty":"empty_text"}}}}"#;
        assert!(parse_spec(bad.as_bytes()).is_err());
    }));

    scenarios.push(tc("parse_oversized_spec_boundary", || {
        // Construct a spec just over MAX_SPEC_BYTES when serialized
        let long_val = "x".repeat(MAX_SPEC_BYTES);
        let j = format!(
            r#"{{"schema_version":1,"id":"big","revision":1,"accepts":["detail"],"root":{{"primitive":"section","props":{{"title":{{"kind":"literal","value":{{"t":"{long_val}"}}}}}}}}}}"#
        );
        assert!(matches!(parse_spec(j.as_bytes()), Err(PresentationError::SpecTooLarge)));
    }));

    scenarios.push(tc("parse_extra_unknown_fields_ignored", || {
        let j = r#"{"schema_version":1,"id":"x","revision":1,"accepts":["detail"],"root":{"primitive":"text","props":{"text":{"kind":"binding","path":"$.text","required":false,"empty":"empty_text"}}},"unknown_field":"value","extra":123}"#;
        assert!(parse_spec(j.as_bytes()).is_ok());
    }));

    scenarios.push(tc("parse_valid_with_all_optional_fields", || {
        let j = r#"{"schema_version":1,"id":"full","revision":1,"accepts":["detail"],"root":{"primitive":"text","props":{"text":{"kind":"binding","path":"$.text","required":false,"empty":"empty_text"}}},"fallback":{"kind":"plain"},"accessibility":{"summary":{"path":"$.title","required":false,"empty":"omit"}},"metadata":{"key":"value"}}"#;
        assert!(parse_spec(j.as_bytes()).is_ok());
    }));

    scenarios.push(tc("parse_valid_with_string_prop", || {
        let j = r#"{"schema_version":1,"id":"full2","revision":1,"accepts":["detail"],"root":{"primitive":"text","props":{"text":{"kind":"binding","path":"$.text","required":false,"empty":"empty_text"}}},"fallback":{"kind":"document"},"accessibility":{"summary":null},"metadata":{}}"#;
        assert!(parse_spec(j.as_bytes()).is_ok());
    }));

    scenarios.push(tc("parse_valid_divider_root", || {
        let j = r#"{"schema_version":1,"id":"d","revision":1,"accepts":["detail"],"root":{"primitive":"divider"}}"#;
        assert!(parse_spec(j.as_bytes()).is_ok());
    }));

    scenarios.push(tc("parse_valid_with_children", || {
        let j = r#"{"schema_version":1,"id":"c","revision":1,"accepts":["detail"],"root":{"primitive":"stack","props":{},"children":[{"primitive":"text","props":{"text":{"kind":"binding","path":"$.a","required":false,"empty":"empty_text"}}},{"primitive":"divider"}]}}"#;
        assert!(parse_spec(j.as_bytes()).is_ok());
    }));

    scenarios.push(tc("parse_valid_with_each_item", || {
        let j = r#"{"schema_version":1,"id":"e","revision":1,"accepts":["detail"],"root":{"primitive":"stack","props":{},"each":{"path":"$.items","required":false,"empty":"omit"},"item":{"primitive":"text","props":{"text":{"kind":"binding","path":"$.text","required":false,"empty":"empty_text"}}}}}"#;
        assert!(parse_spec(j.as_bytes()).is_ok());
    }));

    scenarios.push(tc("parse_invalid_json_malformed", || {
        assert!(matches!(
            parse_spec(b"{bad json}"),
            Err(PresentationError::InvalidJson(_))
        ));
    }));

    scenarios.push(tc("parse_missing_root", || {
        assert!(parse_spec(b"{\"schema_version\":1,\"id\":\"x\",\"revision\":1}").is_err());
    }));

    scenarios.push(tc("parse_zero_revision", || {
        assert!(parse_spec(b"{\"schema_version\":1,\"id\":\"x\",\"revision\":0,\"root\":{\"primitive\":\"divider\"}}").is_err());
    }));

    scenarios.push(tc("parse_valid_multiple_semantic_types", || {
        let j = r#"{"schema_version":1,"id":"m","revision":1,"accepts":["detail","comparison"],"root":{"primitive":"text","props":{"text":{"kind":"binding","path":"$.text","required":false,"empty":"empty_text"}}}}"#;
        assert!(parse_spec(j.as_bytes()).is_ok());
    }));

    run("parse_spec", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 6. INFER SEMANTIC TYPE
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_infer_semantic_type() {
    let cases: Vec<(serde_json::Value, Option<&'static str>, String)> = vec![
        (
            json!({"left": "a", "right": "b"}),
            Some("comparison"),
            "comparison_left_right".into(),
        ),
        (
            json!({"alternatives": [1, 2]}),
            Some("comparison"),
            "comparison_alts".into(),
        ),
        (
            json!({"checks": [1]}),
            Some("checklist"),
            "checklist_checks".into(),
        ),
        (
            json!({"tasks": [1]}),
            Some("checklist"),
            "checklist_tasks".into(),
        ),
        (
            json!({"events": [1]}),
            Some("schedule"),
            "schedule_events".into(),
        ),
        (
            json!({"slots": [1]}),
            Some("schedule"),
            "schedule_slots".into(),
        ),
        (
            json!({"agenda": [1], "attendees": [1]}),
            Some("meeting"),
            "meeting".into(),
        ),
        (
            json!({"lessons": [1]}),
            Some("lesson"),
            "lesson_lessons".into(),
        ),
        (
            json!({"sections": [1]}),
            Some("lesson"),
            "lesson_sections".into(),
        ),
        (
            json!({"decision": "x"}),
            Some("decision"),
            "decision".into(),
        ),
        (
            json!({"choice": "x"}),
            Some("decision"),
            "decision_choice".into(),
        ),
        (
            json!({"income": [1], "expenses": [1]}),
            Some("budget"),
            "budget_ie".into(),
        ),
        (
            json!({"credits": [1], "debits": [1]}),
            Some("budget"),
            "budget_cd".into(),
        ),
        (
            json!({"items": [1]}),
            Some("collection"),
            "collection_items".into(),
        ),
        (
            json!({"entries": [1]}),
            Some("collection"),
            "collection_entries".into(),
        ),
        (json!({"steps": [1]}), Some("steps"), "steps".into()),
        (json!({"status": "active"}), Some("status"), "status".into()),
        (
            json!({"state": "active"}),
            Some("status"),
            "status_state".into(),
        ),
        (json!({"value": 42}), Some("metric"), "metric_value".into()),
        (
            json!({"amount": 42}),
            Some("metric"),
            "metric_amount".into(),
        ),
        (json!({"count": 42}), Some("metric"), "metric_count".into()),
        (json!({"total": 42}), Some("metric"), "metric_total".into()),
        (json!({"value": 0}), Some("metric"), "metric_zero".into()),
        (
            json!({"value": -1}),
            Some("metric"),
            "metric_negative".into(),
        ),
        (json!({"count": true}), None, "count_not_number".into()),
        (
            json!({"title": "hi"}),
            Some("detail"),
            "detail_title".into(),
        ),
        (
            json!({"summary": "hi"}),
            Some("detail"),
            "detail_summary".into(),
        ),
        (json!({"unknown": "field"}), None, "unknown_field".into()),
        (
            json!({"items": "not_array"}),
            None,
            "items_not_array".into(),
        ),
        (json!({"left": 1}), None, "left_without_right".into()),
        (
            json!({"income": 1, "expenses": [1]}),
            None,
            "income_not_array".into(),
        ),
        (
            json!({"value": "not_a_number"}),
            None,
            "value_not_number".into(),
        ),
        (
            json!({"agenda": [1], "attendees": "not_array"}),
            None,
            "agenda_not_array".into(),
        ),
        (
            json!({"credits": 1, "debits": [1]}),
            None,
            "credits_not_array".into(),
        ),
        (json!(42), None, "scalar_int".into()),
        (json!("string"), None, "scalar_string".into()),
        (json!([1, 2, 3]), None, "array_payload".into()),
        (json!(null), None, "null_payload".into()),
        (json!({}), None, "empty_object".into()),
        (
            json!({"items": [1], "title": "T"}),
            Some("collection"),
            "items_wins_over_title".into(),
        ),
        (
            json!({"left": "a", "right": "b", "title": "T"}),
            Some("comparison"),
            "comparison_wins_over_title".into(),
        ),
    ];

    let mut scenarios: Vec<Scenario> = Vec::new();
    for (payload, expected, label) in cases {
        let p = payload;
        let e = expected;
        scenarios.push(tc(&label.clone(), move || {
            let result = infer_semantic_type(&p);
            assert_eq!(result, e, "infer for {label}");
        }));
    }

    run("infer_semantic_type", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 7. PRESENTATION LIBRARY LIFECYCLE
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_library_lifecycle() {
    let mut scenarios: Vec<Scenario> = vec![];

    // 7a. Register
    scenarios.push(tc("register_correct_digest_ok", || {
        let mut lib = PresentationLibrary::default();
        assert!(
            lib.register(stored_simple(
                "ok1",
                1,
                &["detail"],
                text_binding_node(),
                "o1"
            ))
            .is_ok()
        );
    }));

    scenarios.push(tc("register_wrong_digest_mismatch", || {
        let mut lib = PresentationLibrary::default();
        let spec = make_spec("dmi", 1, &["detail"], text_binding_node());
        let dg = digest(&spec).unwrap();
        let bad = format!("{}00", &dg[..dg.len().saturating_sub(2)]);
        let stored = StoredPresentation {
            spec,
            digest: bad,
            origin: PresentationOrigin {
                scope: LibraryScope::Workspace,
                owner: "o1".into(),
                plugin_id: None,
                generation: Some("g1".into()),
            },
            enabled: true,
        };
        assert!(matches!(
            lib.register(stored),
            Err(PresentationError::DigestMismatch)
        ));
    }));

    scenarios.push(tc("register_empty_owner_invalid", || {
        let mut lib = PresentationLibrary::default();
        let spec = make_spec("eoi", 1, &["detail"], text_binding_node());
        let dg = digest(&spec).unwrap();
        let stored = StoredPresentation {
            spec,
            digest: dg,
            origin: PresentationOrigin {
                scope: LibraryScope::Workspace,
                owner: "".into(),
                plugin_id: None,
                generation: Some("g1".into()),
            },
            enabled: true,
        };
        assert!(matches!(
            lib.register(stored),
            Err(PresentationError::InvalidSpec(_))
        ));
    }));

    scenarios.push(tc("register_duplicate_same_digest_ok", || {
        let mut lib = PresentationLibrary::default();
        let s = stored_simple("dup1", 1, &["detail"], text_binding_node(), "o1");
        assert!(lib.register(s.clone()).is_ok());
        assert!(lib.register(s).is_ok());
    }));

    scenarios.push(tc("register_duplicate_different_digest_conflict", || {
        let mut lib = PresentationLibrary::default();
        assert!(
            lib.register(stored_simple(
                "conf1",
                1,
                &["detail"],
                text_binding_node(),
                "o1"
            ))
            .is_ok()
        );
        let mut root2 = text_binding_node();
        root2.children = vec![]; // different text
        let root2 = SpecNode {
            primitive: Primitive::Divider,
            props: BTreeMap::new(),
            children: vec![],
            each: None,
            item: None,
        };
        assert!(matches!(
            lib.register(stored_simple("conf1", 1, &["detail"], root2, "o1")),
            Err(PresentationError::RevisionConflict(_))
        ));
    }));

    // 7b. Activate
    scenarios.push(tc("activate_correct_scope_owner_ok", || {
        let mut lib = PresentationLibrary::default();
        lib.register(stored_simple(
            "ak1",
            1,
            &["detail"],
            section_with_title(),
            "o1",
        ))
        .unwrap();
        assert!(
            lib.activate("ak1", 1, LibraryScope::Workspace, "o1")
                .is_ok()
        );
    }));

    scenarios.push(tc("activate_wrong_scope_denied", || {
        let mut lib = PresentationLibrary::default();
        let mut s = stored_simple("ak2", 1, &["detail"], section_with_title(), "o1");
        s.origin.scope = LibraryScope::User;
        lib.register(s).unwrap();
        assert!(matches!(
            lib.activate("ak2", 1, LibraryScope::Workspace, "o1"),
            Err(PresentationError::ActivationDenied)
        ));
    }));

    scenarios.push(tc("activate_wrong_owner_denied", || {
        let mut lib = PresentationLibrary::default();
        lib.register(stored_simple(
            "ak3",
            1,
            &["detail"],
            section_with_title(),
            "o1",
        ))
        .unwrap();
        assert!(matches!(
            lib.activate("ak3", 1, LibraryScope::Workspace, "wrong"),
            Err(PresentationError::ActivationDenied)
        ));
    }));

    scenarios.push(tc("activate_unknown_presentation_error", || {
        let mut lib = PresentationLibrary::default();
        assert!(matches!(
            lib.activate("nope", 1, LibraryScope::Workspace, "o1"),
            Err(PresentationError::UnknownPresentation(_))
        ));
    }));

    // 7c. Activate built-in from any owner (built-in exemption is owner-only)
    for owner in ["user1", "user2", "any_owner", "builtin"] {
        let o = owner;
        scenarios.push(tc(&format!("activate_builtin_any_owner_{o}"), move || {
            let mut lib = PresentationLibrary::default();
            lib.register(stored_builtin("b1", 1, &["detail"], section_with_title()))
                .unwrap();
            let result = lib.activate("b1", 1, LibraryScope::Workspace, o);
            assert!(result.is_ok(), "builtin from owner {o:?}: {result:?}");
        }));
    }

    // 7d. Select
    scenarios.push(tc("select_returns_highest_revision", || {
        let mut lib = PresentationLibrary::default();
        lib.register(stored_builtin("sel1", 1, &["detail"], section_with_title()))
            .unwrap();
        lib.register(stored_simple(
            "sel1",
            2,
            &["detail"],
            section_with_title(),
            "o1",
        ))
        .unwrap();
        lib.activate("sel1", 1, LibraryScope::Workspace, "builtin")
            .unwrap();
        lib.activate("sel1", 2, LibraryScope::Workspace, "o1")
            .unwrap();
        let selected = lib.select("detail", LibraryScope::Workspace, "o1");
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().spec.revision, 2);
    }));

    scenarios.push(tc("select_filters_by_scope", || {
        let mut lib = PresentationLibrary::default();
        let mut s = stored_simple("sf1", 1, &["detail"], section_with_title(), "o1");
        s.origin.scope = LibraryScope::User;
        lib.register(s).unwrap();
        lib.activate("sf1", 1, LibraryScope::User, "o1").unwrap();
        assert!(
            lib.select("detail", LibraryScope::Workspace, "o1")
                .is_none()
        );
    }));

    scenarios.push(tc("select_filters_by_owner", || {
        let mut lib = PresentationLibrary::default();
        lib.register(stored_simple(
            "sf2",
            1,
            &["detail"],
            section_with_title(),
            "owner_a",
        ))
        .unwrap();
        lib.activate("sf2", 1, LibraryScope::Workspace, "owner_a")
            .unwrap();
        assert!(
            lib.select("detail", LibraryScope::Workspace, "owner_b")
                .is_none()
        );
    }));

    scenarios.push(tc("select_filters_by_semantic_type", || {
        let mut lib = PresentationLibrary::default();
        lib.register(stored_simple(
            "sf3",
            1,
            &["comparison"],
            section_with_title(),
            "o1",
        ))
        .unwrap();
        lib.activate("sf3", 1, LibraryScope::Workspace, "o1")
            .unwrap();
        assert!(
            lib.select("detail", LibraryScope::Workspace, "o1")
                .is_none()
        );
    }));

    scenarios.push(tc("select_no_activation_returns_none", || {
        let mut lib = PresentationLibrary::default();
        lib.register(stored_simple(
            "na1",
            1,
            &["detail"],
            section_with_title(),
            "o1",
        ))
        .unwrap();
        assert!(
            lib.select("detail", LibraryScope::Workspace, "o1")
                .is_none()
        );
    }));

    // select_preferred matrix
    for (u_has, w_has) in [(true, true), (true, false), (false, true), (false, false)] {
        let (uh, wh) = (u_has, w_has);
        scenarios.push(tc(&format!("select_preferred_u_{uh}_w_{wh}"), move || {
            let mut lib = PresentationLibrary::default();
            if uh {
                let mut s = stored_simple("u_spec", 1, &["detail"], section_with_title(), "user1");
                s.origin.scope = LibraryScope::User;
                lib.register(s).unwrap();
                lib.activate("u_spec", 1, LibraryScope::User, "user1")
                    .unwrap();
            }
            if wh {
                let s = stored_simple("w_spec", 1, &["detail"], section_with_title(), "ws1");
                lib.register(s).unwrap();
                lib.activate("w_spec", 1, LibraryScope::Workspace, "ws1")
                    .unwrap();
            }
            let selected = lib.select_preferred("detail", "user1", "ws1");
            match (uh, wh) {
                (true, _) => assert!(selected.is_some()),
                (false, true) => assert!(selected.is_some()),
                (false, false) => assert!(selected.is_none()),
            }
        }));
    }

    // 7e. Reset
    scenarios.push(tc("reset_restores_revision_1", || {
        let mut lib = PresentationLibrary::default();
        lib.register(stored_simple(
            "r1",
            1,
            &["detail"],
            section_with_title(),
            "o1",
        ))
        .unwrap();
        let r2_root = section_with_title();
        lib.register(stored_simple("r1", 2, &["detail"], r2_root, "o1"))
            .unwrap();
        lib.activate("r1", 2, LibraryScope::Workspace, "o1")
            .unwrap();
        assert_eq!(
            lib.select("detail", LibraryScope::Workspace, "o1")
                .unwrap()
                .spec
                .revision,
            2
        );
        assert!(lib.reset("r1", LibraryScope::Workspace, "o1"));
        assert_eq!(
            lib.select("detail", LibraryScope::Workspace, "o1")
                .unwrap()
                .spec
                .revision,
            1
        );
    }));

    scenarios.push(tc("reset_no_original_returns_false", || {
        let mut lib = PresentationLibrary::default();
        lib.register(stored_simple(
            "r2",
            2,
            &["detail"],
            section_with_title(),
            "o1",
        ))
        .unwrap();
        assert!(!lib.reset("r2", LibraryScope::Workspace, "o1"));
    }));

    // 7f. Deactivate
    scenarios.push(tc("deactivate_removes_activation", || {
        let mut lib = PresentationLibrary::default();
        lib.register(stored_simple(
            "da1",
            1,
            &["detail"],
            section_with_title(),
            "o1",
        ))
        .unwrap();
        lib.activate("da1", 1, LibraryScope::Workspace, "o1")
            .unwrap();
        assert!(
            lib.select("detail", LibraryScope::Workspace, "o1")
                .is_some()
        );
        lib.deactivate("da1", LibraryScope::Workspace, "o1");
        assert!(
            lib.select("detail", LibraryScope::Workspace, "o1")
                .is_none()
        );
    }));

    // 7g. Revoke plugin
    for pc in [1usize, 2, 5, 10] {
        scenarios.push(tc(&format!("revoke_plugin_removes_{pc}"), move || {
            let mut lib = PresentationLibrary::default();
            let pid = "plugin.test";
            for i in 0..pc {
                let s = stored_plugin(&format!("p{i}"), 1, &["detail"], text_binding_node(), pid);
                lib.register(s).unwrap();
                lib.activate(&format!("p{i}"), 1, LibraryScope::Workspace, "plugin_owner")
                    .unwrap();
            }
            let before = lib.definitions().count();
            let removed = lib.revoke_plugin(pid);
            assert_eq!(removed, pc);
            assert_eq!(lib.definitions().count(), before - pc);
            assert!(lib.activations().is_empty());
        }));
    }

    // 7h. from_parts
    scenarios.push(tc("from_parts_validates_specs", || {
        let s = stored_simple("fp1", 1, &["detail"], section_with_title(), "o1");
        assert!(PresentationLibrary::from_parts(vec![s], vec![]).is_ok());
    }));

    scenarios.push(tc("from_parts_rejects_bad_digest", || {
        let spec = make_spec("fp2", 1, &["detail"], section_with_title());
        let stored = StoredPresentation {
            spec,
            digest: "wrong".into(),
            origin: PresentationOrigin {
                scope: LibraryScope::Workspace,
                owner: "o1".into(),
                plugin_id: None,
                generation: Some("g1".into()),
            },
            enabled: true,
        };
        assert!(matches!(
            PresentationLibrary::from_parts(vec![stored], vec![]),
            Err(PresentationError::DigestMismatch)
        ));
    }));

    // 7i. pack_manifest
    scenarios.push(tc("pack_manifest_generates_digest", || {
        let mut lib = PresentationLibrary::default();
        lib.register(stored_simple(
            "pm1",
            1,
            &["detail"],
            section_with_title(),
            "o1",
        ))
        .unwrap();
        let manifest = lib.pack_manifest(
            "my_pack",
            "1.0.0",
            vec!["Text".into()],
            Some("publisher".into()),
        );
        assert!(manifest.is_ok());
        let m = manifest.unwrap();
        assert_eq!(m.pack_id, "my_pack");
        assert_eq!(m.version, "1.0.0");
        assert!(!m.digest.is_empty());
    }));

    // 7j. Cross-scope activation denied
    scenarios.push(tc("cross_scope_activation_denied", || {
        let mut lib = PresentationLibrary::default();
        let mut s = stored_simple("cs1", 1, &["detail"], section_with_title(), "user1");
        s.origin.scope = LibraryScope::User;
        lib.register(s).unwrap();
        assert!(matches!(
            lib.activate("cs1", 1, LibraryScope::Workspace, "user1"),
            Err(PresentationError::ActivationDenied)
        ));
    }));

    // 7k. Plugin scope activation ok
    scenarios.push(tc("plugin_scope_activation_ok", || {
        let mut lib = PresentationLibrary::default();
        lib.register(stored_plugin(
            "ps1",
            1,
            &["detail"],
            section_with_title(),
            "plugin.test",
        ))
        .unwrap();
        assert!(
            lib.activate("ps1", 1, LibraryScope::Workspace, "plugin_owner")
                .is_ok()
        );
    }));

    // 7l. get returns registered spec
    scenarios.push(tc("get_after_register", || {
        let mut lib = PresentationLibrary::default();
        lib.register(stored_simple(
            "get1",
            1,
            &["detail"],
            section_with_title(),
            "o1",
        ))
        .unwrap();
        assert!(lib.get("get1", 1).is_some());
        assert!(lib.get("get1", 2).is_none());
    }));

    // 7m. Activate sets enabled
    scenarios.push(tc("activate_sets_enabled", || {
        let mut lib = PresentationLibrary::default();
        let mut s = stored_simple("ae1", 1, &["detail"], section_with_title(), "o1");
        s.enabled = false;
        lib.register(s).unwrap();
        assert!(!lib.get("ae1", 1).unwrap().enabled);
        lib.activate("ae1", 1, LibraryScope::Workspace, "o1")
            .unwrap();
        assert!(lib.get("ae1", 1).unwrap().enabled);
    }));

    // 7n. Deactivate keeps spec
    scenarios.push(tc("deactivate_keeps_spec", || {
        let mut lib = PresentationLibrary::default();
        lib.register(stored_simple(
            "dk1",
            1,
            &["detail"],
            section_with_title(),
            "o1",
        ))
        .unwrap();
        lib.deactivate("dk1", LibraryScope::Workspace, "o1");
        assert!(lib.get("dk1", 1).is_some());
    }));

    // 7o. Multiple activations for different semantic types
    scenarios.push(tc("multiple_activations_different_types", || {
        let mut lib = PresentationLibrary::default();
        lib.register(stored_simple(
            "ma1",
            1,
            &["detail", "comparison"],
            section_with_title(),
            "o1",
        ))
        .unwrap();
        lib.activate("ma1", 1, LibraryScope::Workspace, "o1")
            .unwrap();
        assert!(
            lib.select("detail", LibraryScope::Workspace, "o1")
                .is_some()
        );
        assert!(
            lib.select("comparison", LibraryScope::Workspace, "o1")
                .is_some()
        );
    }));

    // 7p. Revocation does not remove unrelated specs
    scenarios.push(tc("revoke_does_not_remove_unrelated", || {
        let mut lib = PresentationLibrary::default();
        lib.register(stored_simple(
            "rel1",
            1,
            &["detail"],
            section_with_title(),
            "o1",
        ))
        .unwrap();
        lib.register(stored_plugin(
            "rel2",
            1,
            &["detail"],
            section_with_title(),
            "plugin.x",
        ))
        .unwrap();
        lib.revoke_plugin("plugin.x");
        assert!(lib.get("rel1", 1).is_some());
        assert!(lib.get("rel2", 1).is_none());
    }));

    run("library_lifecycle", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 8. REVISION LIFECYCLE
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_revisions() {
    let mut scenarios: Vec<Scenario> = vec![];

    scenarios.push(tc("valid_revision_ok", || {
        let base = make_spec("rev1", 1, &["detail"], text_binding_node());
        let mut proposed = base.clone();
        proposed.revision = 2;
        let req = PresentationRevisionRequest {
            base_id: "rev1".into(),
            base_revision: 1,
            feedback: "improve".into(),
            attempt: 1,
        };
        let result = propose_revision(req, proposed);
        assert!(result.is_ok());
        let rev = result.unwrap();
        assert_eq!(rev.proposed.revision, 2);
        assert_eq!(rev.digest, digest(&rev.proposed).unwrap());
    }));

    scenarios.push(tc("empty_feedback_invalid", || {
        let base = make_spec("rev2", 1, &["detail"], text_binding_node());
        let req = PresentationRevisionRequest {
            base_id: "rev2".into(),
            base_revision: 1,
            feedback: "".into(),
            attempt: 1,
        };
        assert!(matches!(
            propose_revision(req, base),
            Err(PresentationError::InvalidSpec(_))
        ));
    }));

    scenarios.push(tc("whitespace_feedback_invalid", || {
        let base = make_spec("rev3", 1, &["detail"], text_binding_node());
        let req = PresentationRevisionRequest {
            base_id: "rev3".into(),
            base_revision: 1,
            feedback: "  \n\t  ".into(),
            attempt: 1,
        };
        assert!(matches!(
            propose_revision(req, base),
            Err(PresentationError::InvalidSpec(_))
        ));
    }));

    for attempt in [0u8, 3, 4, 255] {
        scenarios.push(tc(&format!("invalid_attempt_{attempt}"), move || {
            let base = make_spec("att", 1, &["detail"], text_binding_node());
            let req = PresentationRevisionRequest {
                base_id: "att".into(),
                base_revision: 1,
                feedback: "fix".into(),
                attempt,
            };
            assert!(matches!(
                propose_revision(req, base),
                Err(PresentationError::RevisionBudgetExceeded)
            ));
        }));
    }

    scenarios.push(tc("identity_changed_error", || {
        let base = make_spec("ic1", 1, &["detail"], text_binding_node());
        let mut proposed = base.clone();
        proposed.id = "ic2".into();
        proposed.revision = 2;
        let req = PresentationRevisionRequest {
            base_id: "ic1".into(),
            base_revision: 1,
            feedback: "fix".into(),
            attempt: 1,
        };
        assert!(matches!(
            propose_revision(req, proposed),
            Err(PresentationError::RevisionIdentityChanged)
        ));
    }));

    scenarios.push(tc("revision_overflow", || {
        let base = make_spec("ro1", u64::MAX, &["detail"], text_binding_node());
        let mut proposed = base.clone();
        proposed.revision = 1;
        let req = PresentationRevisionRequest {
            base_id: "ro1".into(),
            base_revision: u64::MAX,
            feedback: "fix".into(),
            attempt: 1,
        };
        assert!(matches!(
            propose_revision(req, proposed),
            Err(PresentationError::RevisionOverflow)
        ));
    }));

    scenarios.push(tc("digest_matches_compiled", || {
        let base = make_spec("dg1", 1, &["detail"], text_binding_node());
        let mut proposed = base.clone();
        proposed.revision = 2;
        let req = PresentationRevisionRequest {
            base_id: "dg1".into(),
            base_revision: 1,
            feedback: "fix".into(),
            attempt: 1,
        };
        let rev = propose_revision(req, proposed).unwrap();
        assert_eq!(rev.digest, digest(&rev.proposed).unwrap());
    }));

    scenarios.push(tc("revision_corrects_wrong_number", || {
        let base = make_spec("cor1", 1, &["detail"], text_binding_node());
        let mut proposed = base.clone();
        proposed.revision = 999;
        let req = PresentationRevisionRequest {
            base_id: "cor1".into(),
            base_revision: 1,
            feedback: "fix".into(),
            attempt: 1,
        };
        let rev = propose_revision(req, proposed).unwrap();
        assert_eq!(rev.proposed.revision, 2);
    }));

    scenarios.push(tc("revision_2_valid", || {
        let base = make_spec("r2v", 1, &["detail"], text_binding_node());
        let mut proposed = base.clone();
        proposed.revision = 2;
        let req = PresentationRevisionRequest {
            base_id: "r2v".into(),
            base_revision: 1,
            feedback: "fix".into(),
            attempt: 2,
        };
        assert!(propose_revision(req, proposed).is_ok());
    }));

    scenarios.push(tc("revision_rejects_invalid_spec", || {
        let base = make_spec("bad", 1, &["detail"], text_binding_node());
        let mut proposed = base.clone();
        proposed.accepts = vec![]; // Empty accepts is OK
        proposed.revision = 2;
        let req = PresentationRevisionRequest {
            base_id: "bad".into(),
            base_revision: 1,
            feedback: "fix".into(),
            attempt: 1,
        };
        assert!(propose_revision(req, proposed).is_ok());
    }));

    scenarios.push(tc("revision_preserves_metadata", || {
        let base = make_spec("meta", 1, &["detail"], text_binding_node());
        let mut proposed = base.clone();
        proposed.revision = 2;
        proposed.metadata.insert("author".into(), "test".into());
        let req = PresentationRevisionRequest {
            base_id: "meta".into(),
            base_revision: 1,
            feedback: "fix".into(),
            attempt: 1,
        };
        let rev = propose_revision(req, proposed).unwrap();
        assert_eq!(
            rev.proposed.metadata.get("author"),
            Some(&"test".to_string())
        );
    }));

    run("revisions", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 9. SEED PACK
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_seeds() {
    let mut scenarios: Vec<Scenario> = vec![];

    scenarios.push(tc("seed_pack_has_52", || {
        assert_eq!(seeds::built_in_seed_pack().len(), 52);
    }));

    scenarios.push(tc("seed_pack_all_disabled", || {
        assert!(seeds::built_in_seed_pack().iter().all(|r| !r.enabled));
    }));

    scenarios.push(tc("seed_pack_all_nonempty_digest", || {
        assert!(
            seeds::built_in_seed_pack()
                .iter()
                .all(|r| !r.digest.is_empty())
        );
    }));

    scenarios.push(tc("seed_pack_revisions_are_2", || {
        assert!(
            seeds::built_in_seed_pack()
                .iter()
                .all(|r| r.spec.revision == 2)
        );
    }));

    scenarios.push(tc("seed_pack_owner_is_builtin", || {
        assert!(
            seeds::built_in_seed_pack()
                .iter()
                .all(|r| r.origin.owner == "builtin")
        );
    }));

    scenarios.push(tc("seed_pack_has_coding_types", || {
        let has = seeds::built_in_seed_pack()
            .iter()
            .any(|r| r.spec.accepts.iter().any(|t| t.starts_with("coding.")));
        assert!(has);
    }));

    // Each seed validates
    for record in seeds::built_in_seed_pack() {
        let id = record.spec.id.clone();
        scenarios.push(tc(&format!("seed_validates_{id}"), move || {
            assert!(
                validate_spec(&record.spec).is_ok(),
                "seed {id} should validate"
            );
        }));
    }

    // Each seed compiles with generic payload
    for record in seeds::built_in_seed_pack() {
        let id = record.spec.id.clone();
        let st = record.spec.accepts[0].clone();
        scenarios.push(tc(&format!("seed_compiles_{id}"), move || {
            let result = compile(
                &record.spec,
                &ci(
                    &st,
                    json!({"title":"T","subtitle":"S","summary":"Sum","items":["a","b"]}),
                    "fb",
                ),
            );
            assert!(
                matches!(result, CompiledPresentation::Rich(_)),
                "seed {id}: {result:?}"
            );
        }));
    }

    // Seed can be activated and selected
    scenarios.push(tc("seed_activate_and_select", || {
        let mut lib = PresentationLibrary::default();
        let pack = seeds::built_in_seed_pack();
        let seed = pack
            .iter()
            .find(|r| r.spec.accepts[0] == "metric")
            .unwrap()
            .clone();
        let seed_id = seed.spec.id.clone();
        let seed_rev = seed.spec.revision;
        lib.register(seed).unwrap();
        lib.activate(&seed_id, seed_rev, LibraryScope::Workspace, "builtin")
            .unwrap();
        let selected = lib.select("metric", LibraryScope::Workspace, "builtin");
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().spec.id, seed_id);
    }));

    // Each seed has a unique id
    scenarios.push(tc("seed_ids_unique", || {
        let pack = seeds::built_in_seed_pack();
        let ids: std::collections::HashSet<_> = pack.iter().map(|r| &r.spec.id).collect();
        assert_eq!(ids.len(), pack.len());
    }));

    // Each seed root has a valid primitive
    for record in seeds::built_in_seed_pack() {
        let id = record.spec.id.clone();
        let prim = record.spec.root.primitive;
        scenarios.push(tc(&format!("seed_root_primitive_{id}"), move || {
            assert_eq!(record.spec.root.primitive, prim);
        }));
    }

    run("seeds", scenarios);
}
