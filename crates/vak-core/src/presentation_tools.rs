//! `emit_*_card` tools: function-calling based presentation card emission.
//!
//! Measured root cause (2026-09-18, against the real local model this app
//! ships with, `gemma4:e2b-mlx` via Ollama's OpenAI-compatible endpoint at
//! `http://localhost:11434/v1`, the exact endpoint `vak-llm`'s "ollama"
//! provider uses):
//!
//!   - Free-text `vak` fences embedded in prose: 1/5 syntactically valid JSON.
//!   - Tool-calling with a precise, per-shape JSON Schema as the function's
//!     `parameters`: 5/5 valid AND exact shape match, repeated across
//!     multiple categories.
//!   - Tool-calling with one generic/loose schema: worse than fences in one
//!     dimension (the model sometimes emits no tool call and no text at
//!     all) and the payload shape still drifted. Precision in the schema is
//!     what buys the reliability, not tool-calling by itself.
//!
//! So: one tool per real payload *shape* (not per `semantic_type` — many
//! semantic types share an identical shape, e.g. every timeline-flavored
//! type), each with a schema precise enough to match what the client
//! renderer actually reads. These are the same 12 shape categories already
//! used to build the client-side render harness
//! (`crates/vak-client-ui/src/harness/fixtures.ts` `CATEGORY_FIXTURES`) —
//! kept in sync deliberately, since both are describing the same
//! `build*Spec` functions in `GenericSpecRenderer.tsx`.
//!
//! Rendering integration needs no new code: `execute()` just echoes its own
//! (schema-valid-by-construction) arguments back as
//! `{"semantic_type":...,"payload":...}` JSON text. The existing
//! `vak_delivery::structured_outputs_from_tool_result_with` pipeline
//! (originally built for third-party/MCP tool results that self-declare a
//! `semantic_type`) already scans any tool's result for exactly this shape,
//! validates it against `SkillRegistry` — the real 84-type allowlist,
//! independent of anything the model claims — and turns it into a rendered
//! card. That pipeline is untouched; this just gives it schema-clean input
//! instead of a hand-rolled markdown fence.

use async_trait::async_trait;
use serde_json::Value;
use vak_tools::context::ToolContext;
use vak_tools::{Tool, ToolOutput};

struct CardShape {
    /// Tool name, e.g. `emit_chart_card`.
    name: &'static str,
    description: &'static str,
    /// Every `semantic_type` this shape can produce. The model picks one
    /// via the `semantic_type` enum in the schema; `SkillRegistry` (the
    /// real, current type list) re-validates it server-side regardless of
    /// what the model sends, so this list drifting from skills.rs over
    /// time fails closed (rejected, not silently accepted) rather than
    /// open.
    semantic_types: &'static [&'static str],
    /// JSON Schema for the `payload` field specifically (the tool's
    /// `parameters` wraps this as `{semantic_type: enum, payload: <this>}`).
    payload_schema: fn() -> Value,
}

fn universal_card_payload_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "title": {"type": "string"},
            "summary": {"type": "string"}
        },
        "additionalProperties": true,
        "description": "Free-form key/value fields beyond title/summary are shown as a details list."
    })
}

fn research_payload_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "title": {"type": "string"},
            "sources": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "title": {"type": "string"},
                        "url": {"type": "string"},
                        "snippet": {"type": "string"},
                        "source_name": {"type": "string"},
                        "published_at": {"type": "string"}
                    },
                    "required": ["title", "url"]
                }
            },
            "takeaways": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string"},
                        "citation_indices": {
                            "type": "array",
                            "items": {"type": "integer"},
                            "minItems": 1,
                            "description": "1-based indices into `sources`; every takeaway must cite at least one source."
                        }
                    },
                    "required": ["text", "citation_indices"]
                }
            }
        },
        "required": ["sources", "takeaways"]
    })
}

fn diff_payload_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "files": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "filename": {"type": "string"},
                        "additions": {"type": "integer", "minimum": 0},
                        "deletions": {"type": "integer", "minimum": 0},
                        "hunks": {"type": "string", "description": "Unified diff hunk text for this file"}
                    },
                    "required": ["filename", "hunks", "additions", "deletions"]
                }
            }
        },
        "required": ["files"]
    })
}

fn test_matrix_payload_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "suite_name": {"type": "string"},
            "total": {"type": "integer"},
            "passed": {"type": "integer"},
            "failed": {"type": "integer"},
            "skipped": {"type": "integer"},
            "tests": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "status": {"type": "string", "enum": ["passed", "failed", "skipped"]},
                        "duration_ms": {"type": "integer"},
                        "message": {"type": "string"}
                    },
                    "required": ["name", "status"]
                }
            }
        },
        "required": ["tests"]
    })
}

fn terminal_payload_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "command": {"type": "string"},
            "output": {"type": "string"},
            "exit_code": {"type": "integer"},
            "duration_ms": {"type": "integer"}
        },
        "required": ["output"]
    })
}

fn table_payload_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "title": {"type": "string"},
            "columns": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {"key": {"type": "string"}, "label": {"type": "string"}, "isNumeric": {"type": "boolean"}},
                    "required": ["key", "label"]
                }
            },
            "rows": {
                "type": "array",
                "items": {"type": "object", "additionalProperties": true}
            }
        },
        "required": ["columns", "rows"]
    })
}

fn timeline_payload_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "title": {"type": "string"},
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "label": {"type": "string"},
                        "detail": {"type": "string"},
                        "status": {"type": "string"}
                    },
                    "required": ["label"]
                }
            }
        },
        "required": ["items"]
    })
}

fn recipe_payload_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "title": {"type": "string"},
            "servings": {"type": "integer"},
            "prep_time_minutes": {"type": "integer"},
            "cook_time_minutes": {"type": "integer"},
            "ingredients": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {"name": {"type": "string"}, "amount": {"type": "number"}, "unit": {"type": "string"}},
                    "required": ["name"]
                }
            },
            "steps": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string"},
                        "timer_seconds": {"type": "integer", "minimum": 1, "maximum": 86400}
                    },
                    "required": ["text"]
                }
            }
        },
        "required": ["title", "ingredients", "steps"]
    })
}

fn ui_preview_payload_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "status": {"type": "string"},
            "title": {"type": "string"},
            "artifact_path": {"type": "string"},
            "html": {"type": "string"}
        }
    })
}

fn chart_payload_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "title": {"type": "string"},
            "chart_type": {"type": "string", "enum": ["line", "bar", "area"]},
            "x_label": {"type": "string"},
            "y_label": {"type": "string"},
            "accessible_summary": {"type": "string"},
            "series": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "points": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "x": {"type": ["string", "number"]},
                                    "y": {"type": "number"}
                                },
                                "required": ["x", "y"]
                            }
                        }
                    },
                    "required": ["name", "points"]
                }
            }
        },
        "required": ["chart_type", "series", "accessible_summary"]
    })
}

fn media_payload_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "description": "For semantic_type `link.preview`, use the first shape (url+title). For `media.image`/`media.video`/`media.audio`, use the second shape (source+media_type+alt).",
        "oneOf": [
            {
                "properties": {
                    "url": {"type": "string"},
                    "title": {"type": "string"},
                    "image_url": {"type": "string"},
                    "description": {"type": "string"},
                    "site_name": {"type": "string"}
                },
                "required": ["url", "title"]
            },
            {
                "properties": {
                    "source": {"type": "string"},
                    "media_type": {"type": "string", "enum": ["image", "video", "audio"]},
                    "alt": {"type": "string"},
                    "title": {"type": "string"}
                },
                "required": ["source", "media_type", "alt"]
            }
        ]
    })
}

fn metric_payload_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "label": {"type": "string"},
            "value": {"type": ["string", "number"]},
            "unit": {"type": "string"},
            "location": {"type": "string", "description": "Optional grid title when reporting multiple metrics at once"}
        },
        "additionalProperties": {"type": ["string", "number"]},
        "description": "For a single metric, set label/value/unit. For several at once (e.g. a weather or telemetry grid), use additional key/value fields instead."
    })
}

const SHAPES: &[CardShape] = &[
    CardShape {
        name: "emit_universal_card",
        description: "Emit a general-purpose card (map, calendar, board, entity, document, graph, form, alert, and similar) with a title/summary and free-form key/value fields.",
        semantic_types: &[
            "map", "route_map", "calendar", "availability", "board", "entity", "search_results",
            "coding.search", "evidence", "document", "graph", "form", "action", "transaction",
            "alert", "conversation", "progress_dashboard", "simulation",
        ],
        payload_schema: universal_card_payload_schema,
    },
    CardShape {
        name: "emit_research_card",
        description: "Emit a research/news synthesis card with cited sources and takeaways. Use whenever you searched the web or another source and are reporting findings — every takeaway should be traceable to a source.",
        semantic_types: &["research.synthesis", "research_brief", "news"],
        payload_schema: research_payload_schema,
    },
    CardShape {
        name: "emit_diff_card",
        description: "Emit a code-diff card for changes you made.",
        semantic_types: &["coding.diff"],
        payload_schema: diff_payload_schema,
    },
    CardShape {
        name: "emit_test_report_card",
        description: "Emit a test-results card summarizing a test run you actually executed.",
        semantic_types: &["test.report"],
        payload_schema: test_matrix_payload_schema,
    },
    CardShape {
        name: "emit_terminal_card",
        description: "Emit a card showing a command you ran and its real captured output.",
        semantic_types: &["terminal.view"],
        payload_schema: terminal_payload_schema,
    },
    CardShape {
        name: "emit_table_card",
        description: "Emit a data table / comparison / budget / inventory card with explicit columns and rows.",
        semantic_types: &[
            "coding.benchmark", "coding.dependencies", "data.grid", "table", "dataframe",
            "comparison", "comparison_table", "pros_cons", "inventory", "scorecard",
            "budget", "finance_summary", "invoice_summary", "travel_options",
        ],
        payload_schema: table_payload_schema,
    },
    CardShape {
        name: "emit_timeline_card",
        description: "Emit a timeline/plan/checklist/schedule card: an ordered or grouped list of steps, milestones, or items.",
        semantic_types: &[
            "coding.deployment", "coding.incident", "coding.architecture", "coding.release",
            "plan.timeline", "timeline", "itinerary", "checklist", "schedule", "agenda", "milestones",
            "progress", "status", "steps", "overview", "summary", "detail", "notes", "follow_up",
            "reminder", "shopping_list", "lesson", "reading_list", "habit_plan", "project_plan",
            "meeting_notes", "contact_log", "home_project", "care_plan", "event_plan", "media_list",
            "collection", "faq", "decision", "decision_analysis", "meal_plan",
        ],
        payload_schema: timeline_payload_schema,
    },
    CardShape {
        name: "emit_recipe_card",
        description: "Emit a recipe card with ingredients and steps.",
        semantic_types: &["recipe.card", "recipe", "recipe_summary"],
        payload_schema: recipe_payload_schema,
    },
    CardShape {
        name: "emit_ui_preview_card",
        description: "Emit a preview card for an HTML/UI artifact you wrote to the workspace.",
        semantic_types: &["ui.preview"],
        payload_schema: ui_preview_payload_schema,
    },
    CardShape {
        name: "emit_chart_card",
        description: "Emit a chart card for a numeric series over time or categories.",
        semantic_types: &["chart", "trend", "timeseries", "bar_chart"],
        payload_schema: chart_payload_schema,
    },
    CardShape {
        name: "emit_media_card",
        description: "Emit a link preview or media (image/video/audio) card.",
        semantic_types: &["link.preview", "media.image", "media.video", "media.audio"],
        payload_schema: media_payload_schema,
    },
    CardShape {
        name: "emit_metric_card",
        description: "Emit a metric card: a single measurement (weather, a KPI, a benchmark number) or a small grid of several.",
        semantic_types: &["metric"],
        payload_schema: metric_payload_schema,
    },
];

/// Repair payload shapes that `SkillRegistry::validate` checks strictly but
/// that a shared per-*shape* schema can't fully pin down on its own (a
/// handful of the ~84 semantic_types have stricter per-field-name or
/// derived-value requirements than their sibling types in the same shape
/// category). Fixing these here — deterministically, from data the model
/// already gave us — is more reliable than asking a small local model to
/// track field-name synonyms or keep derived counts consistent by hand.
fn normalize_payload(semantic_type: &str, mut payload: Value) -> Value {
    match semantic_type {
        // `itinerary` validates each item's `title`; the shared timeline
        // schema asks the model for `label`. Same data, two field names.
        "itinerary" => {
            if let Some(items) = payload.get_mut("items").and_then(Value::as_array_mut) {
                for item in items {
                    if item.get("title").is_none()
                        && let Some(label) = item.get("label").cloned()
                    {
                        item["title"] = label;
                    }
                }
            }
        }
        // `plan.timeline` validates a top-level `title`; not required by the
        // shared schema since most sibling timeline types don't need one.
        "plan.timeline" => {
            if payload.get("title").and_then(Value::as_str).is_none() {
                payload["title"] = Value::String("Plan".into());
            }
        }
        // `test.report` validates that any `total`/`passed`/`failed`/`skipped`
        // counts the model supplies match the actual `tests` array — so
        // derive them instead of trusting the model to keep them in sync.
        "test.report" => {
            if let Some(tests) = payload.get("tests").and_then(Value::as_array).cloned() {
                let count_where = |status: &str| {
                    tests
                        .iter()
                        .filter(|t| t.get("status").and_then(Value::as_str) == Some(status))
                        .count() as u64
                };
                payload["total"] = Value::from(tests.len() as u64);
                payload["passed"] = Value::from(count_where("passed"));
                payload["failed"] = Value::from(count_where("failed"));
                payload["skipped"] = Value::from(count_where("skipped"));
            }
        }
        _ => {}
    }
    payload
}

pub struct EmitCardTool {
    shape: &'static CardShape,
}

impl EmitCardTool {
    /// One `EmitCardTool` per registered shape category — call this once
    /// per entry in `SHAPES` when assembling the tool list for a turn.
    pub fn all() -> Vec<Self> {
        SHAPES.iter().map(|shape| EmitCardTool { shape }).collect()
    }
}

#[async_trait]
impl Tool for EmitCardTool {
    fn name(&self) -> &str {
        self.shape.name
    }

    fn description(&self) -> &str {
        self.shape.description
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "semantic_type": {
                    "type": "string",
                    "enum": self.shape.semantic_types,
                    "description": "Which of this shape's card types this is."
                },
                "payload": (self.shape.payload_schema)()
            },
            "required": ["semantic_type", "payload"]
        })
    }

    async fn execute(&self, args: &Value, _ctx: &ToolContext) -> ToolOutput {
        let Some(semantic_type) = args.get("semantic_type").and_then(|v| v.as_str()) else {
            return ToolOutput::error("missing or non-string `semantic_type`");
        };
        if !self.shape.semantic_types.contains(&semantic_type) {
            return ToolOutput::error(format!(
                "`{semantic_type}` is not one of this tool's supported types: {:?}. Call the matching emit_*_card tool instead.",
                self.shape.semantic_types
            ));
        }
        let Some(payload) = args.get("payload") else {
            return ToolOutput::error("missing `payload`");
        };
        let payload = normalize_payload(semantic_type, payload.clone());
        // `structured_outputs_from_tool_result_with` (vak-delivery) scans this
        // tool's own result text for a bare `{"semantic_type","payload"}`
        // envelope — no markdown fence needed. `parse_fragment_with` parses
        // that envelope with `#[serde(deny_unknown_fields)]`, so it must be
        // *exactly* these two fields; schema_version/skill_id/skill_version
        // are filled in by the parser itself from the registry, not carried
        // in the envelope.
        let envelope = serde_json::json!({
            "semantic_type": semantic_type,
            "payload": payload,
        });
        ToolOutput::ok(envelope.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shape_has_a_unique_tool_name() {
        let names: std::collections::HashSet<&str> = SHAPES.iter().map(|s| s.name).collect();
        assert_eq!(names.len(), SHAPES.len(), "tool names must be unique");
    }

    #[test]
    fn every_shape_has_at_least_one_semantic_type() {
        for shape in SHAPES {
            assert!(!shape.semantic_types.is_empty(), "{} has no semantic types", shape.name);
        }
    }

    #[test]
    fn no_semantic_type_is_claimed_by_two_shapes() {
        let mut seen = std::collections::HashMap::new();
        for shape in SHAPES {
            for &t in shape.semantic_types {
                if let Some(prev) = seen.insert(t, shape.name) {
                    panic!("semantic_type `{t}` claimed by both {prev} and {}", shape.name);
                }
            }
        }
    }

    #[tokio::test]
    async fn execute_wraps_payload_in_a_findable_envelope() {
        let tool = EmitCardTool::all().into_iter().find(|t| t.name() == "emit_chart_card").unwrap();
        let args = serde_json::json!({
            "semantic_type": "chart",
            "payload": {
                "chart_type": "line",
                "accessible_summary": "flat line",
                "series": [{"name": "s1", "points": [{"x": 1, "y": 2.0}]}]
            }
        });
        let ctx = ToolContext::new(std::env::temp_dir());
        let out = tool.execute(&args, &ctx).await;
        assert!(!out.is_error, "expected Ok, got: {}", out.content);
        let found = vak_delivery::structured_outputs_from_text(&out.content);
        assert_eq!(found.len(), 1, "the render pipeline must find exactly one card in: {}", out.content);
        assert_eq!(found[0].semantic_type, "chart");
    }

    #[tokio::test]
    async fn execute_rejects_a_semantic_type_outside_its_own_shape() {
        let tool = EmitCardTool::all().into_iter().find(|t| t.name() == "emit_chart_card").unwrap();
        let args = serde_json::json!({
            "semantic_type": "recipe.card",
            "payload": {}
        });
        let ctx = ToolContext::new(std::env::temp_dir());
        let out = tool.execute(&args, &ctx).await;
        assert!(out.is_error, "a chart tool must refuse a recipe type");
    }

    /// One representative-but-minimal fixture payload per shape, built to
    /// satisfy that shape's `payload_schema` (and, where the schema alone
    /// isn't enough, the stricter per-type validators in
    /// `vak_delivery::skills::validate_payload`). Every `semantic_type` this
    /// shape's tool can emit is then executed with the SAME fixture, to
    /// prove the shared schema (plus `normalize_payload` for the couple of
    /// known type-specific exceptions) genuinely renders for every type the
    /// tool claims to support — not just one hand-picked example.
    fn fixture_for(shape_name: &str) -> Value {
        match shape_name {
            "emit_universal_card" => serde_json::json!({"title": "T", "summary": "S"}),
            "emit_research_card" => serde_json::json!({
                "sources": [{"title": "Src", "url": "https://example.com"}],
                "takeaways": [{"text": "Point", "citation_indices": [1]}]
            }),
            "emit_diff_card" => serde_json::json!({
                "files": [{"filename": "a.rs", "hunks": "@@ -1 +1 @@", "additions": 1, "deletions": 0}]
            }),
            "emit_test_report_card" => serde_json::json!({
                "tests": [{"name": "it_works", "status": "passed"}]
            }),
            "emit_terminal_card" => serde_json::json!({"command": "ls", "output": "a.rs"}),
            "emit_table_card" => serde_json::json!({
                "columns": [{"key": "name", "label": "Name"}],
                "rows": [{"name": "Alice"}]
            }),
            "emit_timeline_card" => serde_json::json!({
                "title": "T",
                "items": [{"label": "Step 1", "detail": "d"}]
            }),
            "emit_recipe_card" => serde_json::json!({
                "title": "Soup",
                "ingredients": [{"name": "Water"}],
                "steps": [{"text": "Boil"}]
            }),
            "emit_ui_preview_card" => serde_json::json!({"title": "Preview"}),
            "emit_chart_card" => serde_json::json!({
                "chart_type": "line",
                "accessible_summary": "flat",
                "series": [{"name": "s1", "points": [{"x": 1, "y": 2.0}]}]
            }),
            "emit_media_card" => serde_json::json!({"url": "https://example.com", "title": "Link"}),
            "emit_metric_card" => serde_json::json!({"label": "Uptime", "value": 99.9, "unit": "%"}),
            other => panic!("no fixture defined for shape {other} — add one"),
        }
    }

    /// A shape's schema can be a `oneOf` covering several distinct payload
    /// shapes for different semantic_types within it (e.g. `emit_media_card`:
    /// `link.preview` wants url+title, `media.*` wants source+media_type+alt).
    /// Override the shared fixture for those specific types.
    fn fixture_override(semantic_type: &str) -> Option<Value> {
        match semantic_type {
            "media.image" => Some(serde_json::json!({"source": "https://example.com/a.png", "media_type": "image", "alt": "a"})),
            "media.video" => Some(serde_json::json!({"source": "https://example.com/a.mp4", "media_type": "video", "alt": "a"})),
            "media.audio" => Some(serde_json::json!({"source": "https://example.com/a.mp3", "media_type": "audio", "alt": "a"})),
            _ => None,
        }
    }

    #[tokio::test]
    async fn every_registered_semantic_type_across_all_shapes_renders() {
        let mut failures = Vec::new();
        for tool in EmitCardTool::all() {
            let shared_fixture = fixture_for(tool.name());
            for &semantic_type in tool.shape.semantic_types {
                let fixture = fixture_override(semantic_type).unwrap_or_else(|| shared_fixture.clone());
                let args = serde_json::json!({"semantic_type": semantic_type, "payload": fixture.clone()});
                let ctx = ToolContext::new(std::env::temp_dir());
                let out = tool.execute(&args, &ctx).await;
                if out.is_error {
                    failures.push(format!("{semantic_type} ({}): tool rejected: {}", tool.name(), out.content));
                    continue;
                }
                let found = vak_delivery::structured_outputs_from_text(&out.content);
                if found.len() != 1 || found[0].semantic_type != semantic_type {
                    failures.push(format!(
                        "{semantic_type} ({}): render pipeline found {found:?} in {}",
                        tool.name(),
                        out.content
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} of the registered semantic_types failed to render through the real SkillRegistry:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}
