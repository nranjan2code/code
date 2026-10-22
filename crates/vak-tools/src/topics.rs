//! Agent-owned work grouping. The tool carries only its wire contract; the
//! Agent loop answers it with the TopicStore bound to the admitted session.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{Fault, Tool, ToolContext, ToolOutput};
use vak_session::work_threads::TopicId;

pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopicRequest {
    List {
        query: Option<String>,
        offset: usize,
        limit: usize,
    },
    Open {
        id: String,
        offset: usize,
        limit: usize,
    },
    Create {
        title: String,
        pinned: bool,
        outcomes: Vec<String>,
    },
    Rename {
        id: String,
        title: String,
    },
    Pin {
        id: String,
        pinned: bool,
    },
    Archive {
        id: String,
    },
    Reopen {
        id: String,
    },
    Automatic {
        enabled: bool,
    },
    Assign {
        outcome: String,
        topic_ids: Vec<String>,
    },
}

/// Parse the model-facing Topics request. Search is only a display-text
/// filter for finding a topic; it never assigns history or chooses intent.
pub fn parse_topics_args(args: &Value) -> Result<TopicRequest, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("action is required")?;
    let text = |key: &'static str| -> Result<String, String> {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| format!("{key} must be a non-empty string"))
    };
    let id = || -> Result<String, String> {
        let value = text("topic_id")?;
        TopicId::parse(&value).map_err(|_| "topic_id must be a Topics id".to_string())?;
        Ok(value)
    };
    match action {
        "list" => {
            let query = args
                .get("query")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|q| !q.is_empty())
                .map(str::to_owned);
            if query.as_ref().is_some_and(|q| q.chars().count() > 256) {
                return Err("query must be at most 256 characters".into());
            }
            let offset = args
                .get("offset")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(0);
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(DEFAULT_LIMIT)
                .clamp(1, MAX_LIMIT);
            Ok(TopicRequest::List {
                query,
                offset,
                limit,
            })
        }
        "open" => Ok(TopicRequest::Open {
            id: id()?,
            offset: args
                .get("offset")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(0),
            limit: args
                .get("limit")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(DEFAULT_LIMIT)
                .clamp(1, MAX_LIMIT),
        }),
        "create" => {
            let title = text("title")?;
            if title.chars().count() > 160 {
                return Err("title must be at most 160 characters".into());
            }
            let outcomes = match args.get("outcomes") {
                None => Vec::new(),
                Some(Value::Array(values)) if values.len() <= 20 => {
                    let mut outcomes = Vec::with_capacity(values.len());
                    for value in values {
                        let outcome = value
                            .as_str()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .ok_or("each outcomes entry must be non-empty text")?;
                        if outcome.chars().count() > 240 {
                            return Err("each outcome must be at most 240 characters".into());
                        }
                        if !outcomes.iter().any(|known| known == outcome) {
                            outcomes.push(outcome.to_owned());
                        }
                    }
                    outcomes
                }
                Some(Value::Array(_)) => {
                    return Err("outcomes must contain at most 20 entries".into());
                }
                Some(_) => return Err("outcomes must be a list of outcome labels".into()),
            };
            Ok(TopicRequest::Create {
                title,
                pinned: args.get("pinned").and_then(Value::as_bool).unwrap_or(false),
                outcomes,
            })
        }
        "rename" => {
            let title = text("title")?;
            if title.chars().count() > 160 {
                return Err("title must be at most 160 characters".into());
            }
            Ok(TopicRequest::Rename { id: id()?, title })
        }
        "pin" => Ok(TopicRequest::Pin {
            id: id()?,
            pinned: args
                .get("pinned")
                .and_then(Value::as_bool)
                .ok_or("pinned must be true or false")?,
        }),
        "archive" => Ok(TopicRequest::Archive { id: id()? }),
        "reopen" => Ok(TopicRequest::Reopen { id: id()? }),
        "automatic" => Ok(TopicRequest::Automatic {
            enabled: args
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or("enabled must be true or false")?,
        }),
        "assign" => {
            let outcome = text("outcome")?;
            if outcome.chars().count() > 240 {
                return Err("outcome must be at most 240 characters".into());
            }
            let ids = args
                .get("topic_ids")
                .and_then(Value::as_array)
                .ok_or("topic_ids must be a list")?;
            if ids.len() > 20 {
                return Err("topic_ids must contain at most 20 topics".into());
            }
            let mut topic_ids = Vec::with_capacity(ids.len());
            for value in ids {
                let value = value
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or("each topic_ids entry must be a Topics id")?;
                TopicId::parse(value)
                    .map_err(|_| "each topic_ids entry must be a Topics id".to_string())?;
                if !topic_ids.iter().any(|known| known == value) {
                    topic_ids.push(value.to_owned());
                }
            }
            Ok(TopicRequest::Assign { outcome, topic_ids })
        }
        _ => Err(
            "action must be list, open, create, rename, pin, archive, reopen, automatic, or assign"
                .into(),
        ),
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TopicsTool;

#[async_trait]
impl Tool for TopicsTool {
    fn name(&self) -> &str {
        "topics"
    }
    fn description(&self) -> &str {
        "Organize work into Topics when it helps. List pages through all topics, including older ones; open a topic to see its paged turn addresses and placements. Create or rename topics, pin them, and assign requested outcomes from this turn to one or more topics. When creating a topic for this turn, include its requested outcomes in `outcomes` to create and place them together; use assign for an existing topic. If a person asks to use an archived Topic, explain that it is archived and ask them to reopen it in Topics controls; do not create a substitute to bypass that choice. Assignment groups records only; it does not load their contents into context. For a simple, self-contained answer while automatic organisation is on, record an explicit ungrouped decision with assign, an outcome label, and an empty topic_ids list."
    }
    fn schema(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["list","open","create","rename","pin","assign"]},
            "query":{"type":"string","description":"Optional display-text filter for list; it does not classify work."},
            "offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":100},
            "topic_id":{"type":"string","description":"A Topics id returned by list or open."},
            "title":{"type":"string"},"pinned":{"type":"boolean"},"enabled":{"type":"boolean"},
            "outcomes":{"type":"array","description":"Optional requested outcomes from this turn to place in this newly created topic in the same operation.","maxItems":20,"items":{"type":"string","maxLength":240}},
            "outcome":{"type":"string","description":"One requested outcome from the current turn, described in a few words."},
            "topic_ids":{"type":"array","description":"Topics for this outcome; an empty list explicitly leaves a simple independent answer ungrouped.","items":{"type":"string"},"maxItems":20}
        },"required":["action"]})
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::error(
            Fault::Tool,
            "Topics is a broker-owned tool and cannot run in a worker.",
        )
    }
    fn always_loaded(&self) -> bool {
        true
    }
    fn looks_up(&self) -> bool {
        false
    }
    fn permission(&self) -> vak_permission::Declared {
        vak_permission::Declared::reach(vak_permission::Reach::Coordination)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_tool_is_always_loaded_and_has_paged_discovery() {
        let tool = TopicsTool;
        assert!(tool.always_loaded());
        let schema = tool.schema();
        assert!(schema["properties"]["offset"].is_object());
        assert!(schema["properties"]["limit"].is_object());
        assert!(schema["properties"]["query"].is_object());
        assert!(
            schema["properties"]["action"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "open")
        );
    }

    #[test]
    fn list_search_is_only_a_bounded_display_filter() {
        let parsed = parse_topics_args(
            &json!({"action":"list","query":"  launch  ","offset":50,"limit":500}),
        )
        .unwrap();
        assert_eq!(
            parsed,
            TopicRequest::List {
                query: Some("launch".into()),
                offset: 50,
                limit: MAX_LIMIT
            }
        );
    }

    #[test]
    fn create_accepts_optional_current_turn_outcomes() {
        assert_eq!(
            parse_topics_args(
                &json!({"action":"create","title":"Draft","outcomes":["Outline","Review"]})
            )
            .unwrap(),
            TopicRequest::Create {
                title: "Draft".into(),
                pinned: false,
                outcomes: vec!["Outline".into(), "Review".into()]
            }
        );
        assert!(
            parse_topics_args(&json!({"action":"create","title":"Draft","outcomes":[""]})).is_err()
        );
        assert!(
            parse_topics_args(
                &json!({"action":"create","title":"Draft","outcomes":["Outline"],"pinned":true})
            )
            .is_ok()
        );
    }

    #[test]
    fn assignment_requires_real_topic_ids_and_outcome_text() {
        assert!(
            parse_topics_args(&json!({"action":"assign","outcome":"Draft","topic_ids":["wrong"]}))
                .is_err()
        );
        assert!(parse_topics_args(&json!({"action":"assign","topic_ids":[]})).is_err());
        assert_eq!(
            parse_topics_args(
                &json!({"action":"assign","outcome":"Independent answer","topic_ids":[]})
            )
            .unwrap(),
            TopicRequest::Assign {
                outcome: "Independent answer".into(),
                topic_ids: vec![]
            }
        );
        assert!(parse_topics_args(&json!({"action":"assign","outcome":"Draft","topic_ids":["top_018f0f1e-7b3a-7abc-8def-0123456789ab"]})).is_ok());
    }
}
