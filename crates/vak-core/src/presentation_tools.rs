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
//! Call and response, not echo: the model's `emit_*_card` call carries the
//! card in its arguments (schema-constrained, and recorded untruncated in the
//! ledger). `execute()` validates them against the real `SkillRegistry` and
//! answers with a short ack — or, if the card is invalid, a tool error the
//! model can repair in the same turn. `vak-server`'s projection rebuilds the
//! card from the call arguments via `card_output_from_call`. The card is never
//! carried in the result text: the tool framework line-truncates results at
//! ~2000 characters, which silently destroyed any larger card (found live with
//! a research card), and echoing the data back also invited the model to
//! restate it as a duplicate fence.

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
            "map",
            "route_map",
            "calendar",
            "availability",
            "board",
            "entity",
            "search_results",
            "coding.search",
            "evidence",
            "document",
            "graph",
            "form",
            "action",
            "transaction",
            "alert",
            "conversation",
            "progress_dashboard",
            "simulation",
        ],
        payload_schema: universal_card_payload_schema,
    },
    CardShape {
        name: "emit_research_card",
        description: "Emit a research/news synthesis card: several distinct findings drawn from multiple cited sources, each takeaway traceable to a source. Choose it by the shape of the answer, not because you searched: a single measurement or fact (a temperature, a price, a score) belongs on the metric card and a comparison on the table card, with the source named in your sentence.",
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
            "coding.benchmark",
            "coding.dependencies",
            "data.grid",
            "table",
            "dataframe",
            "comparison",
            "comparison_table",
            "pros_cons",
            "inventory",
            "scorecard",
            "budget",
            "finance_summary",
            "invoice_summary",
            "travel_options",
            "decision_matrix",
            "criteria_matrix",
            "tradeoff_analysis",
        ],
        payload_schema: table_payload_schema,
    },
    CardShape {
        name: "emit_timeline_card",
        description: "Emit a timeline/plan/checklist/schedule card: an ordered or grouped list of steps, milestones, or items.",
        semantic_types: &[
            "coding.deployment",
            "coding.incident",
            "coding.architecture",
            "coding.release",
            "plan.timeline",
            "timeline",
            "itinerary",
            "checklist",
            "schedule",
            "agenda",
            "milestones",
            "progress",
            "status",
            "steps",
            "overview",
            "summary",
            "detail",
            "notes",
            "follow_up",
            "reminder",
            "shopping_list",
            "lesson",
            "reading_list",
            "habit_plan",
            "project_plan",
            "meeting_notes",
            "contact_log",
            "home_project",
            "care_plan",
            "event_plan",
            "media_list",
            "collection",
            "faq",
            "decision",
            "decision_analysis",
            "meal_plan",
        ],
        payload_schema: timeline_payload_schema,
    },
    CardShape {
        name: "emit_recipe_card",
        description: "Emit a recipe card with ingredients and steps.",
        semantic_types: &[
            "recipe.card",
            "recipe",
            "recipe_summary",
            "lifestyle.recipe",
            "lifestyle.culinary_recipe",
        ],
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
        semantic_types: &[
            "chart",
            "trend",
            "timeseries",
            "bar_chart",
            "metric_chart",
            "comparison_chart",
            "telemetry.chart",
        ],
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
        description: "Emit a metric card: a single current measurement or a small grid of them (weather, a price, a KPI, a benchmark number). Prefer it whenever the answer is one value, even if you searched the web to get it.",
        semantic_types: &["metric", "telemetry.metric", "weather"],
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

/// One representative-but-minimal fixture payload per shape, built to
/// satisfy that shape's `payload_schema` (and, where the schema alone
/// isn't enough, the stricter per-type validators in
/// `vak_delivery::skills::validate_payload`). Every `semantic_type` this
/// shape's tool can emit is then executed with the SAME fixture, to
/// prove the shared schema (plus `normalize_payload` for the couple of
/// known type-specific exceptions) genuinely renders for every type the
/// tool claims to support — not just one hand-picked example.
#[allow(clippy::panic)]
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
        "emit_metric_card" => {
            serde_json::json!({"label": "Uptime", "value": 99.9, "unit": "%"})
        }
        other => panic!("no fixture defined for shape {other} — add one"),
    }
}

/// A shape's schema can be a `oneOf` covering several distinct payload
/// shapes for different semantic_types within it (e.g. `emit_media_card`:
/// `link.preview` wants url+title, `media.*` wants source+media_type+alt).
/// Override the shared fixture for those specific types.
fn fixture_override(semantic_type: &str) -> Option<Value> {
    match semantic_type {
        "media.image" => Some(
            serde_json::json!({"source": "https://example.com/a.png", "media_type": "image", "alt": "a"}),
        ),
        "media.video" => Some(
            serde_json::json!({"source": "https://example.com/a.mp4", "media_type": "video", "alt": "a"}),
        ),
        "media.audio" => Some(
            serde_json::json!({"source": "https://example.com/a.mp3", "media_type": "audio", "alt": "a"}),
        ),
        _ => None,
    }
}

/// Every `(tool, semantic_type, payload)` the tools claim to support, with a
/// schema-valid payload — the single source for conformance tests here and in
/// vak-server (which checks the full call → ledger → projection path).
#[doc(hidden)]
pub fn conformance_cases() -> Vec<(&'static str, &'static str, Value)> {
    let mut out = Vec::new();
    for shape in SHAPES {
        for &semantic_type in shape.semantic_types {
            let payload =
                fixture_override(semantic_type).unwrap_or_else(|| fixture_for(shape.name));
            out.push((shape.name, semantic_type, payload));
        }
    }
    out
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

    fn presents_cards(&self) -> bool {
        true
    }

    async fn execute(&self, args: &Value, _ctx: &ToolContext) -> ToolOutput {
        match validate_call(self.shape, args, &vak_delivery::built_in_skill_registry()) {
            Ok(output) => ToolOutput::ok(format!(
                "Card displayed to the user ({}). It is already on screen: do not restate its \
                 data or write a `vak` fence for it; at most add one short sentence of narration.",
                output.semantic_type
            )),
            Err(reason) => ToolOutput::error(format!(
                "Card not displayed: {reason}. Fix the arguments and call {} again.",
                self.shape.name
            )),
        }
    }
}

fn validate_call(
    shape: &CardShape,
    args: &Value,
    skills: &vak_delivery::SkillRegistry,
) -> Result<vak_delivery::StructuredOutput, String> {
    let Some(semantic_type) = args.get("semantic_type").and_then(Value::as_str) else {
        return Err("missing or non-string `semantic_type`".into());
    };
    if !shape.semantic_types.contains(&semantic_type) {
        return Err(format!(
            "`{semantic_type}` is not one of this tool's types {:?}; use the matching emit_*_card tool",
            shape.semantic_types
        ));
    }
    let Some(payload) = args.get("payload") else {
        return Err("missing `payload`".into());
    };
    let envelope = serde_json::json!({
        "semantic_type": semantic_type,
        "payload": normalize_payload(semantic_type, payload.clone()),
    });
    vak_delivery::parse_fragment_with(&envelope.to_string(), skills).map_err(|e| e.to_string())
}

/// The `emit_*_card` tool that carries `semantic_type`, if any.
pub fn emit_tool_for(semantic_type: &str) -> Option<&'static str> {
    SHAPES
        .iter()
        .find(|shape| shape.semantic_types.contains(&semantic_type))
        .map(|shape| shape.name)
}

/// The one-shot nudge for an answer that reads as something the app presents
/// as a card but was written as prose. Driven entirely by the app's own signal
/// and recipe detection (`signals_from_text` → `RecipeCatalog::intended_outputs`)
/// — no per-type rules here — and only names a tool that was actually offered.
pub fn presentation_check_nudge(
    text: &str,
    offered_tools: &[String],
    recipes: &vak_delivery::RecipeCatalog,
) -> Option<String> {
    let signals = vak_delivery::signals_from_text(text);
    let intended = recipes.intended_outputs(&signals, "desktop")?;
    let (semantic_type, tool) = intended.primary_types.iter().find_map(|semantic_type| {
        let tool = emit_tool_for(semantic_type)?;
        offered_tools
            .iter()
            .any(|offered| offered == tool)
            .then_some((semantic_type.as_str(), tool))
    })?;
    Some(format!(
        "[presentation-check]: Your answer reads as `{}` (signals: {}), which the app presents \
         as a card, but no card was emitted. If a card fits, call `{tool}` with \
         semantic_type `{semantic_type}` and this content, then add at most one short sentence \
         and do not restate the data as text. If a card genuinely does not fit, resend your \
         answer unchanged.",
        intended.recipe_id,
        intended.matched_signals.join(", ")
    ))
}

/// Whether `name` is one of the `emit_*_card` tools.
pub fn is_card_tool(name: &str) -> bool {
    SHAPES.iter().any(|shape| shape.name == name)
}

/// The card an `emit_*_card` call displays, rebuilt from the call's own
/// arguments — the ledger records these untruncated, so nothing depends on
/// the tool result text (which the tool framework line-truncates at ~2000
/// characters, silently destroying any larger card's JSON).
pub fn card_output_from_call(
    name: &str,
    input: &Value,
    skills: &vak_delivery::SkillRegistry,
) -> Option<vak_delivery::StructuredOutput> {
    let shape = SHAPES.iter().find(|shape| shape.name == name)?;
    validate_call(shape, input, skills).ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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
            assert!(
                !shape.semantic_types.is_empty(),
                "{} has no semantic types",
                shape.name
            );
        }
    }

    #[test]
    fn no_semantic_type_is_claimed_by_two_shapes() {
        let mut seen = std::collections::HashMap::new();
        for shape in SHAPES {
            for &t in shape.semantic_types {
                if let Some(prev) = seen.insert(t, shape.name) {
                    panic!(
                        "semantic_type `{t}` claimed by both {prev} and {}",
                        shape.name
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn a_valid_call_is_acked_and_rebuilds_into_a_card() {
        let tool = EmitCardTool::all()
            .into_iter()
            .find(|t| t.name() == "emit_chart_card")
            .unwrap();
        let args = serde_json::json!({
            "semantic_type": "chart",
            "payload": {
                "chart_type": "line",
                "accessible_summary": "flat line",
                "series": [{"name": "s1", "points": [{"x": 1, "y": 2.0}]}]
            }
        });
        let out = tool
            .execute(&args, &ToolContext::new(std::env::temp_dir()))
            .await;
        assert!(!out.is_error, "expected Ok, got: {}", out.content);
        assert!(out.content.contains("already on screen"), "{}", out.content);
        assert!(
            !out.content.contains("semantic_type"),
            "ack must not echo the card"
        );
        let card = card_output_from_call(
            "emit_chart_card",
            &args,
            &vak_delivery::built_in_skill_registry(),
        )
        .expect("the call's own arguments must rebuild into a card");
        assert_eq!(card.semantic_type, "chart");
    }

    #[tokio::test]
    async fn an_invalid_payload_is_a_repairable_tool_error_not_a_silent_drop() {
        let tool = EmitCardTool::all()
            .into_iter()
            .find(|t| t.name() == "emit_chart_card")
            .unwrap();
        let args = serde_json::json!({
            "semantic_type": "chart",
            "payload": {"chart_type": "line", "series": []}
        });
        let out = tool
            .execute(&args, &ToolContext::new(std::env::temp_dir()))
            .await;
        assert!(
            out.is_error,
            "validator-rejected card must be an error the model sees"
        );
        assert!(out.content.contains("Fix the arguments"), "{}", out.content);
    }

    #[tokio::test]
    async fn a_card_far_larger_than_the_tool_output_line_limit_still_rebuilds() {
        let long = "x".repeat(6000);
        let args = serde_json::json!({
            "semantic_type": "research.synthesis",
            "payload": {
                "sources": [{"title": "S", "url": "https://example.com"}],
                "takeaways": [{"text": long, "citation_indices": [1]}]
            }
        });
        let card = card_output_from_call(
            "emit_research_card",
            &args,
            &vak_delivery::built_in_skill_registry(),
        )
        .expect("size must not matter: the card comes from the call arguments");
        assert_eq!(card.semantic_type, "research.synthesis");
    }

    #[tokio::test]
    async fn execute_rejects_a_semantic_type_outside_its_own_shape() {
        let tool = EmitCardTool::all()
            .into_iter()
            .find(|t| t.name() == "emit_chart_card")
            .unwrap();
        let args = serde_json::json!({
            "semantic_type": "recipe.card",
            "payload": {}
        });
        let ctx = ToolContext::new(std::env::temp_dir());
        let out = tool.execute(&args, &ctx).await;
        assert!(out.is_error, "a chart tool must refuse a recipe type");
    }

    #[tokio::test]
    async fn every_registered_semantic_type_across_all_shapes_renders() {
        let skills = vak_delivery::built_in_skill_registry();
        let mut failures = Vec::new();
        for tool in EmitCardTool::all() {
            for (name, semantic_type, payload) in conformance_cases()
                .into_iter()
                .filter(|(name, _, _)| *name == tool.name())
            {
                let args = serde_json::json!({"semantic_type": semantic_type, "payload": payload});
                let ctx = ToolContext::new(std::env::temp_dir());
                let out = tool.execute(&args, &ctx).await;
                if out.is_error {
                    failures.push(format!(
                        "{semantic_type} ({name}): rejected: {}",
                        out.content
                    ));
                    continue;
                }
                let card = card_output_from_call(name, &args, &skills);
                if card.as_ref().map(|c| c.semantic_type.as_str()) != Some(semantic_type) {
                    failures.push(format!(
                        "{semantic_type} ({name}): did not rebuild into a card"
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} registered types failed:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    #[test]
    fn prose_that_reads_as_a_card_gets_a_nudge_naming_the_offered_tool() {
        let recipes = vak_delivery::built_in_recipes();
        let offered: Vec<String> = EmitCardTool::all()
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        let table = "Weekly moves:\n\n| Index | Change |\n|---|---|\n| Nifty | -0.22% |\n| Sensex | -0.65% |\n";
        let nudge = presentation_check_nudge(table, &offered, &recipes)
            .expect("a markdown table reads as a data grid");
        assert!(
            nudge.contains("emit_table_card") && nudge.contains("data.grid"),
            "{nudge}"
        );
        // no offered tool => no nudge (never tell the model to call something it lacks)
        assert!(presentation_check_nudge(table, &[], &recipes).is_none());
    }

    #[test]
    fn ordinary_conversation_gets_no_nudge() {
        let recipes = vak_delivery::built_in_recipes();
        let offered: Vec<String> = EmitCardTool::all()
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        for text in [
            "Sure, happy to help. What would you like to do next?",
            "The capital of France is Paris.",
            "I renamed the variable and the build passes.",
        ] {
            assert!(
                presentation_check_nudge(text, &offered, &recipes).is_none(),
                "{text}"
            );
        }
    }

    #[test]
    fn every_emit_tool_is_reachable_from_its_types() {
        for shape in SHAPES {
            for t in shape.semantic_types {
                assert_eq!(emit_tool_for(t), Some(shape.name));
            }
        }
    }
}
