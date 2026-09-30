//! `tasks` — model-visible schedule management (docs/design/29-personal-os.md
//! P2). Lets the running agent create, list, edit, and delete this
//! workspace's scheduled routines from a plain-language request ("remind me
//! every weekday at 9am to check the deploy") or a spoken one relayed
//! through the same turn — from the CLI, the desktop app, or any bound chat
//! surface (Telegram/Discord/Slack) alike, since it's just another tool on
//! the same agent loop those all share.
//!
//! Deliberately thin: all validation, cron parsing, and persistence are
//! [`crate::tasks`] (`TaskStore`/`TaskDef`), the exact store the CLI
//! (`vak tasks`) and the server's `/tasks` REST endpoints already share —
//! a task created by any of the three is visible and editable from the
//! other two.

use std::path::PathBuf;

use serde_json::Value;

use crate::tasks::{TaskDef, TaskError, TaskStore};

pub struct TasksTool {
    pub sessions_home: PathBuf,
    pub cwd: PathBuf,
    /// `<surface>:<chat>` for the conversation this turn is running in, if
    /// any (see [`crate::Core::with_default_deliver_to`]). Used only when
    /// the model's `add` call omits `deliver_to` — the common case, since a
    /// user asking for a reminder from inside a chat means "tell me here,"
    /// not "tell nobody."
    pub default_deliver_to: Option<String>,
}

fn task_error_message(e: TaskError) -> String {
    e.to_string()
}

fn nanos_id() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
}

fn render_task(t: &TaskDef) -> Value {
    serde_json::json!({
        "id": t.id,
        "name": t.name,
        "enabled": t.enabled,
        "prompt": if t.prompt.is_empty() { None } else { Some(t.prompt.clone()) },
        "script": t.script,
        "schedule": t.schedule,
        "timezone": t.timezone,
        "due_at": t.due_at,
        "interval_secs": if t.schedule.is_none() { Some(t.interval_secs) } else { None },
        "deliver_to": t.deliver_to,
        "model_pin": t.model_pin,
        "agent_id": t.agent_id,
        "agent_revision": t.agent_revision,
        "last_run_at": t.last_run_at,
        "last_summary": t.last_summary,
        "last_result_id": t.last_result_id,
        "last_run_status": t.last_run_status,
        "last_delivery_state": t.last_delivery_state,
    })
}

#[async_trait::async_trait]
impl vak_tools::Tool for TasksTool {
    fn name(&self) -> &str {
        "tasks"
    }

    fn serves(&self) -> &'static [&'static str] {
        &["orchestration"]
    }

    fn description(&self) -> &str {
        "Manage this workspace's scheduled routines — recurring prompts or \
         shell checks that fire on their own later, without you being asked \
         again. Use for any request to be reminded, checked in on, or have \
         something run on a schedule ('remind me every morning at 8', \
         'check disk space hourly and tell me if it's low', 'run the nightly \
         report'). `action: \"add\"` creates one; give `cron` for a specific \
         schedule (5-field, local time: min hour dom mon dow, e.g. '0 9 * * \
         1-5' for weekdays at 9am) or `every_secs` for a plain interval — \
         exactly one of the two. Give exactly one of `prompt` (a task for \
         you, running as a fresh turn later) or `script` (a one-line shell \
         check; only its output is reported, so a silent/zero-exit check \
         costs nothing). Omit `deliver_to` to have results come back to \
         this same conversation when one is bound; pass it explicitly \
         ('<surface>:<chat>') to route elsewhere. `list` shows every task \
         with its id, schedule, and last result; `enable`/`disable`/`remove` \
         take that id."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "add", "enable", "disable", "remove"],
                    "description": "What to do"
                },
                "id": {
                    "type": "string",
                    "description": "Task id — required for enable/disable/remove, from a prior 'list' or 'add'"
                },
                "name": {
                    "type": "string",
                    "description": "Short label for the task (required for add)"
                },
                "prompt": {
                    "type": "string",
                    "description": "What you should do when this fires, in your own words (XOR with 'script')"
                },
                "script": {
                    "type": "string",
                    "description": "A one-line shell command to run instead of a prompt turn (XOR with 'prompt')"
                },
                "cron": {
                    "type": "string",
                    "description": "5-field cron expression, local time (XOR with 'every_secs')"
                },
                "every_secs": {
                    "type": "integer",
                    "description": "Plain interval in seconds since last run (XOR with 'cron'; default 3600 if neither is given)"
                },
                "timezone": {
                    "type": "string",
                    "description": "Optional IANA timezone name for cron interpretation, such as America/New_York"
                },
                "due_at": {
                    "type": "string",
                    "description": "Optional one-shot UTC timestamp in RFC3339; cannot be combined with cron"
                },
                "deliver_to": {
                    "type": "string",
                    "description": "Where to send the result, as '<surface>:<chat>'. Omit to use the current conversation, when one is bound."
                },
                "model_pin": {
                    "type": "string",
                    "description": "Optional: pin this task to one model id instead of the workspace default"
                },
                "agent": {
                    "type": "string",
                    "description": "Optional saved Agent name or id; records Agent provenance for future runs"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: &Value, _ctx: &vak_tools::ToolContext) -> vak_tools::ToolOutput {
        let mut store = match TaskStore::load(&self.sessions_home) {
            Ok(s) => s,
            Err(e) => return vak_tools::ToolOutput::error(task_error_message(e)),
        };
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        let str_arg = |key: &str| -> Option<String> {
            args.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };

        match action {
            "list" => {
                let mut tasks = store.for_cwd(&self.cwd);
                tasks.sort_by_key(|t| t.created_at);
                if tasks.is_empty() {
                    return vak_tools::ToolOutput::ok(
                        "no scheduled tasks in this workspace yet".to_string(),
                    );
                }
                let rendered: Vec<Value> = tasks.iter().map(render_task).collect();
                vak_tools::ToolOutput::ok(
                    serde_json::to_string_pretty(&rendered).unwrap_or_default(),
                )
            }
            "add" => {
                let Some(name) = str_arg("name") else {
                    return vak_tools::ToolOutput::error("'name' is required for action 'add'");
                };
                let prompt = str_arg("prompt");
                let script = str_arg("script");
                let cron = str_arg("cron");
                let every_secs = args.get("every_secs").and_then(Value::as_u64);
                if cron.is_some() && every_secs.is_some() {
                    return vak_tools::ToolOutput::error("give 'cron' or 'every_secs', not both");
                }
                let (prompt, script) = match (prompt, script) {
                    (Some(p), None) => (p, None),
                    (None, Some(s)) => (String::new(), Some(s)),
                    (None, None) => {
                        return vak_tools::ToolOutput::error(
                            "exactly one of 'prompt' or 'script' is required",
                        );
                    }
                    (Some(_), Some(_)) => {
                        return vak_tools::ToolOutput::error("give 'prompt' or 'script', not both");
                    }
                };
                let deliver_to = str_arg("deliver_to").or_else(|| self.default_deliver_to.clone());
                let agent_id = str_arg("agent");
                let agent_revision = agent_id.as_ref().and_then(|id| {
                    std::fs::read_to_string(self.cwd.join(".vak/agents.json"))
                        .ok()
                        .and_then(|raw| serde_json::from_str::<Vec<Value>>(&raw).ok())
                        .and_then(|profiles| {
                            profiles.into_iter().find(|profile| {
                                profile.get("id").and_then(Value::as_str) == Some(id)
                                    || profile
                                        .get("name")
                                        .and_then(Value::as_str)
                                        .is_some_and(|name| name.eq_ignore_ascii_case(id))
                            })
                        })
                        .and_then(|profile| profile.get("revision").and_then(Value::as_u64))
                });
                if let Some(d) = &deliver_to
                    && !d.contains(':')
                {
                    return vak_tools::ToolOutput::error(
                        "'deliver_to' must be '<surface>:<chat>', e.g. 'telegram:12345'",
                    );
                }
                let task = TaskDef {
                    id: format!("agent-{}", nanos_id()),
                    name,
                    prompt,
                    interval_secs: every_secs.unwrap_or(3600),
                    enabled: true,
                    cwd: self.cwd.clone(),
                    created_at: chrono::Utc::now(),
                    last_run_at: None,
                    last_session_id: None,
                    last_summary: None,
                    last_result_id: None,
                    last_run_status: None,
                    mail_calendar_last_check_at: None,
                    last_delivery_state: None,
                    last_wt: None,
                    deliver_to,
                    schedule: cron,
                    timezone: args
                        .get("timezone")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    due_at: args
                        .get("due_at")
                        .and_then(Value::as_str)
                        .and_then(|value| value.parse().ok()),
                    script,
                    model_pin: str_arg("model_pin"),
                    agent_id,
                    agent_revision,
                    mail_calendar_scope: None,
                };
                if let Err(e) = task.validate() {
                    return vak_tools::ToolOutput::error(task_error_message(e));
                }
                let rendered = render_task(&task);
                store.put(task);
                if let Err(e) = store.save() {
                    return vak_tools::ToolOutput::error(task_error_message(e));
                }
                vak_tools::ToolOutput::ok(format!(
                    "created task {}\n{}",
                    rendered["id"].as_str().unwrap_or_default(),
                    serde_json::to_string_pretty(&rendered).unwrap_or_default()
                ))
            }
            "enable" | "disable" => {
                let Some(id) = str_arg("id") else {
                    return vak_tools::ToolOutput::error(format!(
                        "'id' is required for action '{action}'"
                    ));
                };
                let Some(mut task) = store.get(&id).cloned() else {
                    return vak_tools::ToolOutput::error(format!("no task '{id}'"));
                };
                task.enabled = action == "enable";
                store.put(task);
                if let Err(e) = store.save() {
                    return vak_tools::ToolOutput::error(task_error_message(e));
                }
                vak_tools::ToolOutput::ok(format!(
                    "{} task {id}",
                    if action == "enable" {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }
            "remove" => {
                let Some(id) = str_arg("id") else {
                    return vak_tools::ToolOutput::error("'id' is required for action 'remove'");
                };
                if !store.remove(&id) {
                    return vak_tools::ToolOutput::error(format!("no task '{id}'"));
                }
                if let Err(e) = store.save() {
                    return vak_tools::ToolOutput::error(task_error_message(e));
                }
                vak_tools::ToolOutput::ok(format!("removed task {id}"))
            }
            other => vak_tools::ToolOutput::error(format!(
                "unknown action '{other}'; expected list, add, enable, disable, or remove"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use vak_tools::Tool;

    fn tool(dir: &std::path::Path) -> TasksTool {
        TasksTool {
            sessions_home: dir.to_path_buf(),
            cwd: PathBuf::from("/ws"),
            default_deliver_to: None,
        }
    }

    fn ctx() -> vak_tools::ToolContext {
        vak_tools::ToolContext::new(PathBuf::from("/ws"))
    }

    #[tokio::test]
    async fn add_then_list_round_trips_through_the_shared_store() {
        let dir = tempfile::tempdir().unwrap();
        let t = tool(dir.path());
        let out = t
            .execute(
                &serde_json::json!({
                    "action": "add",
                    "name": "nightly digest",
                    "prompt": "summarize today",
                    "cron": "0 21 * * *"
                }),
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("created task agent-"));

        let out = t
            .execute(&serde_json::json!({"action": "list"}), &ctx())
            .await;
        assert!(!out.is_error);
        assert!(out.content.contains("nightly digest"));
        assert!(out.content.contains("0 21 * * *"));

        // Same file the CLI/server store reads — round-trips unchanged.
        let reloaded = TaskStore::load(dir.path()).unwrap();
        assert_eq!(reloaded.for_cwd(&PathBuf::from("/ws")).len(), 1);
    }

    #[tokio::test]
    async fn add_defaults_deliver_to_from_the_bound_chat() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = tool(dir.path());
        t.default_deliver_to = Some("telegram:12345".to_string());
        let out = t
            .execute(
                &serde_json::json!({
                    "action": "add",
                    "name": "reminder",
                    "prompt": "check in",
                    "every_secs": 3600
                }),
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("telegram:12345"));
    }

    #[tokio::test]
    async fn explicit_deliver_to_overrides_the_chat_default() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = tool(dir.path());
        t.default_deliver_to = Some("telegram:12345".to_string());
        let out = t
            .execute(
                &serde_json::json!({
                    "action": "add",
                    "name": "reminder",
                    "prompt": "check in",
                    "every_secs": 3600,
                    "deliver_to": "log:audit"
                }),
                &ctx(),
            )
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("log:audit"));
        assert!(!out.content.contains("telegram:12345"));
    }

    #[tokio::test]
    async fn cron_and_interval_together_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let t = tool(dir.path());
        let out = t
            .execute(
                &serde_json::json!({
                    "action": "add",
                    "name": "x",
                    "prompt": "p",
                    "cron": "0 9 * * *",
                    "every_secs": 60
                }),
                &ctx(),
            )
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("not both"));
    }

    #[tokio::test]
    async fn prompt_and_script_together_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let t = tool(dir.path());
        let out = t
            .execute(
                &serde_json::json!({
                    "action": "add",
                    "name": "x",
                    "prompt": "p",
                    "script": "echo hi"
                }),
                &ctx(),
            )
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("not both"));
    }

    #[tokio::test]
    async fn bad_cron_is_rejected_before_touching_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let t = tool(dir.path());
        let out = t
            .execute(
                &serde_json::json!({
                    "action": "add",
                    "name": "x",
                    "prompt": "p",
                    "cron": "not a cron"
                }),
                &ctx(),
            )
            .await;
        assert!(out.is_error);
        assert!(!dir.path().join("tasks.json").exists());
    }

    #[tokio::test]
    async fn enable_disable_remove_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let t = tool(dir.path());
        t.execute(
            &serde_json::json!({"action": "add", "name": "x", "script": "true"}),
            &ctx(),
        )
        .await;
        let store = TaskStore::load(dir.path()).unwrap();
        let id = store.for_cwd(&PathBuf::from("/ws"))[0].id.clone();

        let out = t
            .execute(&serde_json::json!({"action": "disable", "id": id}), &ctx())
            .await;
        assert!(!out.is_error);
        let store = TaskStore::load(dir.path()).unwrap();
        assert!(!store.get(&id).unwrap().enabled);

        let out = t
            .execute(&serde_json::json!({"action": "enable", "id": id}), &ctx())
            .await;
        assert!(!out.is_error);
        let store = TaskStore::load(dir.path()).unwrap();
        assert!(store.get(&id).unwrap().enabled);

        let out = t
            .execute(&serde_json::json!({"action": "remove", "id": id}), &ctx())
            .await;
        assert!(!out.is_error);
        let store = TaskStore::load(dir.path()).unwrap();
        assert!(store.get(&id).is_none());
    }

    #[tokio::test]
    async fn unknown_id_is_a_typed_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let t = tool(dir.path());
        let out = t
            .execute(
                &serde_json::json!({"action": "remove", "id": "nope"}),
                &ctx(),
            )
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("no task 'nope'"));
    }
}
