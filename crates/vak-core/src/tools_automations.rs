//! `automations` — the model-facing way to manage this workspace's
//! automations (plan M4.3; docs/design/29-personal-os.md P2): prompts or
//! shell checks that run on their own later, created from a plain-language
//! request ("remind me every weekday at 9am to check the deploy") on any
//! surface, since it is just another tool on the agent loop they share.
//!
//! Deliberately thin: validation, schedules and storage are
//! [`crate::triggers`], the one store the CLI (`vak triggers`) and the
//! server's `/triggers` endpoints share. What an automation's runs did is
//! read from the run records, never stored on it.

use serde_json::Value;

use crate::triggers::{self, Schedule, Trigger, TriggerAction, TriggerKind};

pub struct AutomationsTool {
    pub shared: vak_config::scope::SharedScope,
    pub runs: vak_session::runs::Runs,
    pub cwd: std::path::PathBuf,
    /// The Agent this turn runs as; a new automation is its unless the
    /// call names another saved Agent.
    pub agent: String,
    /// `<surface>:<chat>` for the conversation this turn is running in, if
    /// any (see [`crate::Core::with_default_deliver_to`]). Used only when
    /// the model's `add` call omits `deliver_to`: a person asking for a
    /// reminder from inside a chat means "tell me here", not "tell nobody".
    pub default_deliver_to: Option<String>,
}

fn render(trigger: &Trigger, runs: &vak_session::runs::Runs) -> Value {
    let last = triggers::last_run(runs, &trigger.id).ok().flatten();
    serde_json::json!({
        "id": trigger.id.to_string(),
        "name": trigger.name,
        "enabled": trigger.enabled,
        "agent": trigger.agent,
        "when": trigger.kind,
        "does": trigger.action,
        "deliver_to": trigger.deliver_to,
        "next_run": trigger.next_slot_after(chrono::Utc::now()),
        "last_run": last.map(|run| serde_json::json!({
            "status": run.status,
            "started_at": run.opened_at,
            "reason": run.reason,
        })),
    })
}

/// The revision of the saved Agent named `name` (id or name), when there is
/// one in this workspace.
fn agent_revision(cwd: &std::path::Path, name: &str) -> Option<(String, u64)> {
    let raw =
        std::fs::read_to_string(vak_config::scope::WorkspaceScope::new(cwd).agents_file()).ok()?;
    let profiles = serde_json::from_str::<Vec<Value>>(&raw).ok()?;
    let profile = profiles.into_iter().find(|profile| {
        profile.get("id").and_then(Value::as_str) == Some(name)
            || profile
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
    })?;
    Some((
        profile.get("id").and_then(Value::as_str)?.to_string(),
        profile.get("revision").and_then(Value::as_u64)?,
    ))
}

#[async_trait::async_trait]
impl vak_tools::Tool for AutomationsTool {
    fn name(&self) -> &str {
        "automations"
    }

    fn serves(&self) -> &'static [&'static str] {
        &["orchestration"]
    }

    fn description(&self) -> &str {
        "Manage this workspace's automations: prompts or shell checks that \
         run on their own later, without you being asked again. Use for any \
         request to be reminded, checked in on, or have something run on a \
         schedule ('remind me on Friday at 9', 'every weekday at 8 send me \
         the headlines', 'check disk space hourly and tell me if it's low'). \
         `action: \"add\"` creates one. For a single future moment give \
         `due_at` (an RFC3339 instant: convert the person's local time using \
         the timezone in <turn_context>). For a repeating schedule give \
         `cron` (5-field: min hour dom mon dow, e.g. '0 9 * * 1-5' for \
         weekdays at 9am) with `timezone` set to the person's IANA zone, or \
         `every_secs` for a plain interval: exactly one of the three. When \
         the day or time is ambiguous and it matters, confirm the \
         interpreted date and time before adding. Give exactly one of \
         `prompt` (a task for you, running as a fresh turn later) or \
         `script` (a one-line shell check; only its output is reported, so a \
         silent zero-exit check costs nothing). Omit `deliver_to` to have \
         results come back to this conversation when one is bound; pass it \
         explicitly ('<surface>:<chat>') to route elsewhere. `list` shows \
         every automation with its id, next run and last run; \
         `enable`/`disable`/`remove` take that id."
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
                    "description": "Automation id, required for enable/disable/remove, from a prior 'list' or 'add'"
                },
                "name": {
                    "type": "string",
                    "description": "Short label for the automation (required for add)"
                },
                "prompt": {
                    "type": "string",
                    "description": "What you should do when it runs, in your own words (XOR with 'script')"
                },
                "script": {
                    "type": "string",
                    "description": "A one-line shell command to run instead of a prompt turn (XOR with 'prompt')"
                },
                "cron": {
                    "type": "string",
                    "description": "5-field cron expression for a repeating schedule, read in `timezone` (XOR with 'every_secs' and 'due_at')"
                },
                "every_secs": {
                    "type": "integer",
                    "description": "Plain interval in seconds, from now (XOR with 'cron' and 'due_at'; at least 60)"
                },
                "timezone": {
                    "type": "string",
                    "description": "IANA timezone name the cron is read in, such as Asia/Kolkata or America/New_York; without it the host's zone is used"
                },
                "due_at": {
                    "type": "string",
                    "description": "A single future instant in RFC3339 (with its offset) for a one-time reminder or run; cannot be combined with cron or every_secs"
                },
                "deliver_to": {
                    "type": "string",
                    "description": "Where to send the result, as '<surface>:<chat>'. Omit to use the current conversation, when one is bound."
                },
                "model_pin": {
                    "type": "string",
                    "description": "Optional: pin a prompt automation to one model id instead of the workspace default"
                },
                "agent": {
                    "type": "string",
                    "description": "Optional saved Agent name or id to run it as; defaults to you"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: &Value, _ctx: &vak_tools::ToolContext) -> vak_tools::ToolOutput {
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        let str_arg = |key: &str| -> Option<String> {
            args.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let all = match triggers::list(&self.shared) {
            Ok(all) => triggers::for_workspace(all, &self.cwd),
            Err(e) => return vak_tools::ToolOutput::error(e.to_string()),
        };
        let mine = |id: &str| all.iter().any(|trigger| trigger.id.to_string() == id);

        match action {
            "list" => {
                if all.is_empty() {
                    return vak_tools::ToolOutput::ok(
                        "no automations in this workspace yet".to_string(),
                    );
                }
                let rendered: Vec<Value> = all
                    .iter()
                    .map(|trigger| render(trigger, &self.runs))
                    .collect();
                vak_tools::ToolOutput::ok(
                    serde_json::to_string_pretty(&rendered).unwrap_or_default(),
                )
            }
            "add" => {
                let Some(name) = str_arg("name") else {
                    return vak_tools::ToolOutput::error("'name' is required for action 'add'");
                };
                let action = match (str_arg("prompt"), str_arg("script")) {
                    (Some(text), None) => TriggerAction::Prompt {
                        text,
                        model_pin: str_arg("model_pin"),
                    },
                    (None, Some(command)) => TriggerAction::Script { command },
                    (None, None) => {
                        return vak_tools::ToolOutput::error(
                            "exactly one of 'prompt' or 'script' is required",
                        );
                    }
                    (Some(_), Some(_)) => {
                        return vak_tools::ToolOutput::error("give 'prompt' or 'script', not both");
                    }
                };
                let now = chrono::Utc::now();
                let due_at = match str_arg("due_at") {
                    Some(raw) => match raw.parse::<chrono::DateTime<chrono::Utc>>() {
                        Ok(at) => Some(at),
                        Err(_) => {
                            return vak_tools::ToolOutput::error(
                                "'due_at' must be an RFC3339 instant with its offset",
                            );
                        }
                    },
                    None => None,
                };
                let every_secs = args.get("every_secs").and_then(Value::as_u64);
                let schedule = match (str_arg("cron"), every_secs, due_at) {
                    (Some(expr), None, None) => Schedule::Cron {
                        expr,
                        timezone: str_arg("timezone"),
                    },
                    (None, Some(every_secs), None) => Schedule::Interval {
                        every_secs,
                        anchor: now,
                    },
                    (None, None, Some(at)) => Schedule::Once { at },
                    (None, None, None) => {
                        return vak_tools::ToolOutput::error(
                            "give one of 'cron', 'every_secs' or 'due_at'",
                        );
                    }
                    _ => {
                        return vak_tools::ToolOutput::error(
                            "give only one of 'cron', 'every_secs' or 'due_at'",
                        );
                    }
                };
                let deliver_to = str_arg("deliver_to").or_else(|| self.default_deliver_to.clone());
                if let Some(d) = &deliver_to
                    && !d.contains(':')
                {
                    return vak_tools::ToolOutput::error(
                        "'deliver_to' must be '<surface>:<chat>', e.g. 'telegram:12345'",
                    );
                }
                let (agent, agent_revision) = match str_arg("agent") {
                    Some(named) => match agent_revision(&self.cwd, &named) {
                        Some((id, revision)) => (id, Some(revision)),
                        None => {
                            return vak_tools::ToolOutput::error(format!(
                                "no saved Agent '{named}' in this workspace"
                            ));
                        }
                    },
                    None => (self.agent.clone(), None),
                };
                let trigger = Trigger {
                    id: vak_session::ids::TriggerId::new(),
                    name,
                    agent,
                    agent_revision,
                    space: vak_config::spaces::key(&self.cwd),
                    enabled: true,
                    kind: TriggerKind::Schedule { schedule },
                    action,
                    deliver_to,
                    on_crash: triggers::OnCrash::Skip,
                    scope: None,
                    created_at: now,
                    created_by: None,
                };
                if let Err(e) = triggers::create(&self.shared, &trigger) {
                    return vak_tools::ToolOutput::error(e.to_string());
                }
                vak_tools::ToolOutput::ok(format!(
                    "created automation {}\n{}",
                    trigger.id,
                    serde_json::to_string_pretty(&render(&trigger, &self.runs)).unwrap_or_default()
                ))
            }
            "enable" | "disable" | "remove" => {
                let Some(id) = str_arg("id") else {
                    return vak_tools::ToolOutput::error(format!(
                        "'id' is required for action '{action}'"
                    ));
                };
                if !mine(&id) {
                    return vak_tools::ToolOutput::error(format!(
                        "no automation '{id}' in this workspace"
                    ));
                }
                if action == "remove" {
                    if triggers::get(&self.shared, &id)
                        .ok()
                        .flatten()
                        .is_some_and(|trigger| trigger.source().is_some())
                    {
                        return vak_tools::ToolOutput::error(format!(
                            "automation {id} polls an intake source; remove the source instead"
                        ));
                    }
                    return match triggers::delete(&self.shared, &self.runs, &id) {
                        Ok(_) => vak_tools::ToolOutput::ok(format!("removed automation {id}")),
                        Err(e) => vak_tools::ToolOutput::error(e.to_string()),
                    };
                }
                let enabled = action == "enable";
                match triggers::update(&self.shared, &id, |trigger| {
                    trigger.enabled = enabled;
                    Ok(())
                }) {
                    Ok(_) => vak_tools::ToolOutput::ok(format!(
                        "{} automation {id}",
                        if enabled { "enabled" } else { "disabled" }
                    )),
                    Err(e) => vak_tools::ToolOutput::error(e.to_string()),
                }
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

    fn tool(home: &std::path::Path, cwd: &std::path::Path) -> AutomationsTool {
        AutomationsTool {
            shared: vak_config::scope::SharedScope::new(home),
            runs: vak_session::runs::Runs::at(
                home.join("runs"),
                vak_config::paths::local_tenant_home(),
            ),
            cwd: cwd.to_path_buf(),
            agent: "vak".into(),
            default_deliver_to: Some("telegram:42".into()),
        }
    }

    fn ctx() -> vak_tools::ToolContext {
        vak_tools::ToolContext::new(std::path::PathBuf::from("/"))
    }

    #[tokio::test]
    async fn add_list_disable_remove_round_trip() {
        vak_config::paths::isolate_home_for_tests();
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        vak_config::spaces::bind(cwd.path()).unwrap();
        let tool = tool(home.path(), cwd.path());
        let added = tool
            .execute(
                &serde_json::json!({
                    "action": "add", "name": "standup", "prompt": "post the standup",
                    "cron": "0 9 * * 1-5", "timezone": "Europe/London",
                }),
                &ctx(),
            )
            .await;
        assert!(!added.is_error, "{}", added.content);
        let stored = triggers::list(&tool.shared).unwrap();
        assert_eq!(stored.len(), 1);
        let trigger = &stored[0];
        assert_eq!(trigger.agent, "vak");
        assert_eq!(trigger.deliver_to.as_deref(), Some("telegram:42"));
        let id = trigger.id.to_string();

        let listed = tool
            .execute(&serde_json::json!({"action": "list"}), &ctx())
            .await;
        assert!(listed.content.contains(&id), "{}", listed.content);
        let disabled = tool
            .execute(&serde_json::json!({"action": "disable", "id": id}), &ctx())
            .await;
        assert!(!disabled.is_error, "{}", disabled.content);
        assert!(!triggers::get(&tool.shared, &id).unwrap().unwrap().enabled);
        let removed = tool
            .execute(&serde_json::json!({"action": "remove", "id": id}), &ctx())
            .await;
        assert!(!removed.is_error, "{}", removed.content);
        assert!(triggers::list(&tool.shared).unwrap().is_empty());
    }

    #[tokio::test]
    async fn ambiguous_or_invalid_adds_are_refused_before_storing() {
        vak_config::paths::isolate_home_for_tests();
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        vak_config::spaces::bind(cwd.path()).unwrap();
        let tool = tool(home.path(), cwd.path());
        for args in [
            serde_json::json!({"action": "add", "name": "x", "prompt": "p", "cron": "0 9 * * *", "every_secs": 600}),
            serde_json::json!({"action": "add", "name": "x", "prompt": "p", "script": "true", "every_secs": 600}),
            serde_json::json!({"action": "add", "name": "x", "prompt": "p", "cron": "99 * * * *"}),
            serde_json::json!({"action": "add", "name": "x", "prompt": "p", "every_secs": 5}),
            serde_json::json!({"action": "add", "name": "x", "prompt": "p"}),
            serde_json::json!({"action": "add", "name": "x", "prompt": "p", "every_secs": 600, "deliver_to": "nowhere"}),
        ] {
            let out = tool.execute(&args, &ctx()).await;
            assert!(out.is_error, "{args}");
        }
        assert!(triggers::list(&tool.shared).unwrap().is_empty());
        let unknown = tool
            .execute(
                &serde_json::json!({"action": "remove", "id": "trg_x"}),
                &ctx(),
            )
            .await;
        assert!(unknown.is_error);
    }
}
