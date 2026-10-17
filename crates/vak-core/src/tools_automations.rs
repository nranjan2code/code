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
use vak_tools::Fault;

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
    /// The owner's IANA zone (`Core::owner_timezone`): what a time the
    /// model gives means, and the zone of a repeating schedule made
    /// without one.
    pub owner_zone: String,
    /// Starts a run now where a scheduler runs (`Core::set_trigger_runner`).
    pub runner: Option<std::sync::Arc<dyn triggers::Runner>>,
    /// The Library, where a page an automation keeps current lives.
    pub artifacts: crate::artifacts::Artifacts,
}

fn render(trigger: &Trigger, runs: &vak_session::runs::Runs, zone: &str) -> Value {
    let last = triggers::last_run(runs, &trigger.id).ok().flatten();
    serde_json::json!({
        "id": trigger.id.to_string(),
        "name": trigger.name,
        "paused": !trigger.enabled,
        "agent": trigger.agent,
        "when": trigger.when_words(zone),
        "does": trigger.action,
        "deliver_to": trigger.deliver_to,
        "page": trigger.page,
        "next_run": trigger
            .next_slot_after(chrono::Utc::now())
            .filter(|_| trigger.enabled)
            .map(|at| triggers::local_words(at, zone)),
        "last_run": last.map(|run| serde_json::json!({
            "status": run.status,
            "started_at": triggers::local_words(run.opened_at, zone),
            "reason": run.reason,
        })),
    })
}

/// One line saying what an automation is now, in the words every surface
/// uses.
fn summary(trigger: &Trigger, zone: &str) -> String {
    let next = match trigger.next_slot_after(chrono::Utc::now()) {
        _ if !trigger.enabled => "paused".to_string(),
        Some(at) => format!("next run {}", triggers::local_words(at, zone)),
        None => "no run left".to_string(),
    };
    format!(
        "'{}' ({}): {}; {next}",
        trigger.name,
        trigger.id,
        trigger.when_words(zone)
    )
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

type Args<'a> = &'a Value;

fn str_arg(args: Args<'_>, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

type Refusal = (Fault, String);

fn no(fault: Fault, text: impl Into<String>) -> Refusal {
    (fault, text.into())
}

fn refuse(fault: Fault, text: impl Into<String>) -> vak_tools::ToolOutput {
    vak_tools::ToolOutput::error(fault, text.into())
}

impl AutomationsTool {
    /// What it does, when the call says: `prompt` or `script`, never both.
    fn work(args: Args<'_>) -> Result<Option<TriggerAction>, Refusal> {
        match (str_arg(args, "prompt"), str_arg(args, "script")) {
            (Some(text), None) => Ok(Some(TriggerAction::Prompt {
                text,
                model_pin: str_arg(args, "model_pin"),
            })),
            (None, Some(command)) => Ok(Some(TriggerAction::Script { command })),
            (None, None) => Ok(None),
            (Some(_), Some(_)) => Err(no(Fault::Shape, "give 'prompt' or 'script', not both")),
        }
    }

    /// When it runs, when the call says: one of `at`, `cron`, `every_secs`.
    fn when(&self, args: Args<'_>) -> Result<Option<Schedule>, Refusal> {
        let now = chrono::Utc::now();
        let once = match str_arg(args, "at") {
            Some(raw) => Some(
                triggers::local_instant(&raw, &self.owner_zone, now)
                    .map_err(|reason| (Fault::Shape, reason))?,
            ),
            None => None,
        };
        let every_secs = args.get("every_secs").and_then(Value::as_u64);
        match (str_arg(args, "cron"), every_secs, once) {
            (Some(expr), None, None) => Ok(Some(Schedule::Cron {
                expr,
                timezone: str_arg(args, "timezone"),
            })),
            (None, Some(every_secs), None) => Ok(Some(Schedule::Interval {
                every_secs,
                anchor: now,
            })),
            (None, None, Some(at)) => Ok(Some(Schedule::Once { at })),
            (None, None, None) => Ok(None),
            _ => Err(no(
                Fault::Shape,
                "give only one of 'cron', 'every_secs' or 'at'",
            )),
        }
    }

    fn deliver_to(args: Args<'_>) -> Result<Option<String>, Refusal> {
        match str_arg(args, "deliver_to") {
            Some(target) if !target.contains(':') => Err(no(
                Fault::Shape,
                "'deliver_to' must be '<surface>:<chat>', e.g. 'telegram:12345'",
            )),
            target => Ok(target),
        }
    }

    /// The automation `automation` names, by id or by name; a name that
    /// two share is refused with both.
    fn find<'t>(all: &'t [Trigger], args: Args<'_>) -> Result<&'t Trigger, Refusal> {
        let Some(key) = str_arg(args, "automation") else {
            return Err(no(
                Fault::Shape,
                "'automation' (its name or id) is required for this action",
            ));
        };
        if let Some(trigger) = all.iter().find(|trigger| trigger.id.to_string() == key) {
            return Ok(trigger);
        }
        let named: Vec<&Trigger> = all
            .iter()
            .filter(|trigger| trigger.name.trim().eq_ignore_ascii_case(&key))
            .collect();
        match named.as_slice() {
            [one] => Ok(one),
            [] => Err(no(
                Fault::Target,
                format!(
                    "no automation '{key}' in this workspace; there are: {}",
                    all.iter()
                        .map(|trigger| format!("'{}'", trigger.name))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )),
            many => Err(no(
                Fault::Target,
                format!(
                    "{} automations are named '{key}'; name one by id: {}",
                    many.len(),
                    many.iter()
                        .map(|trigger| trigger.id.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )),
        }
    }

    fn add(&self, args: Args<'_>, ctx: &vak_tools::ToolContext) -> vak_tools::ToolOutput {
        let Some(name) = str_arg(args, "name") else {
            return refuse(Fault::Shape, "'name' is required for action 'add'");
        };
        let action = match Self::work(args) {
            Ok(Some(action)) => action,
            Ok(None) => {
                return refuse(
                    Fault::Shape,
                    "exactly one of 'prompt' or 'script' is required",
                );
            }
            Err((fault, text)) => return refuse(fault, text),
        };
        let schedule = match self.when(args) {
            Ok(Some(schedule)) => schedule,
            Ok(None) => return refuse(Fault::Shape, "give one of 'cron', 'every_secs' or 'at'"),
            Err((fault, text)) => return refuse(fault, text),
        };
        let deliver_to = match Self::deliver_to(args) {
            Ok(target) => target.or_else(|| self.default_deliver_to.clone()),
            Err((fault, text)) => return refuse(fault, text),
        };
        let (agent, agent_revision) = match str_arg(args, "agent") {
            Some(named) => match agent_revision(&self.cwd, &named) {
                Some((id, revision)) => (id, Some(revision)),
                None => {
                    return refuse(
                        Fault::Target,
                        format!("no saved Agent '{named}' in this workspace"),
                    );
                }
            },
            None => (self.agent.clone(), None),
        };
        let mut trigger = Trigger {
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
            created_at: chrono::Utc::now(),
            created_by: None,
            home: ctx
                .trace
                .as_ref()
                .and_then(|trace| trace.session_id())
                .map(|session| triggers::Home {
                    agent: self.agent.clone(),
                    session,
                }),
            made_by: triggers::MadeBy::of_call(ctx.trace.as_ref()),
            page: None,
        };
        trigger.in_zone(&self.owner_zone);
        let new_page = args.get("as_page").and_then(Value::as_bool) == Some(true);
        match str_arg(args, "page") {
            Some(_) if new_page => {
                return refuse(Fault::Shape, "give 'page' or 'as_page', not both");
            }
            Some(page) => match vak_session::ids::ArtifactId::parse(&page) {
                Ok(page) => trigger.page = Some(page),
                Err(_) => {
                    return refuse(
                        Fault::Shape,
                        format!("'{page}' is not a Library item's id; 'recall' finds one"),
                    );
                }
            },
            None => {}
        }
        if let Some(refusal) = trigger.page_refusal(&self.artifacts) {
            return refuse(Fault::Target, refusal);
        }
        if new_page {
            if !matches!(trigger.action, TriggerAction::Prompt { .. }) {
                return refuse(
                    Fault::Shape,
                    "a page is kept current by an automation that runs a prompt",
                );
            }
            match trigger.declare_page(&self.artifacts, None) {
                Ok(page) => trigger.page = Some(page),
                Err(e) => return refuse(Fault::Tool, e.to_string()),
            }
        }
        if let Err(e) = triggers::create(&self.shared, &trigger) {
            return refuse(Fault::Tool, e.to_string());
        }
        let page = trigger.page.map_or_else(String::new, |page| {
            format!("; each run's answer becomes the next version of Library page {page}")
        });
        vak_tools::ToolOutput::ok(format!(
            "created {}{page}",
            summary(&trigger, &self.owner_zone)
        ))
    }

    fn change(
        &self,
        trigger: &Trigger,
        args: Args<'_>,
        ctx: &vak_tools::ToolContext,
    ) -> vak_tools::ToolOutput {
        let (action, schedule, deliver_to) =
            match (Self::work(args), self.when(args), Self::deliver_to(args)) {
                (Ok(action), Ok(schedule), Ok(deliver_to)) => (action, schedule, deliver_to),
                (Err((fault, text)), _, _)
                | (_, Err((fault, text)), _)
                | (_, _, Err((fault, text))) => {
                    return refuse(fault, text);
                }
            };
        let name = str_arg(args, "name");
        if action.is_none() && schedule.is_none() && deliver_to.is_none() && name.is_none() {
            return refuse(
                Fault::Shape,
                "say what changes: 'name', 'prompt' or 'script', a schedule ('at', 'cron' or 'every_secs'), or 'deliver_to'",
            );
        }
        if trigger.source().is_some() || trigger.scope.is_some() {
            return refuse(
                Fault::Refused,
                format!(
                    "'{}' belongs to a source or an email routine; change it there",
                    trigger.name
                ),
            );
        }
        let made_by = triggers::MadeBy::of_call(ctx.trace.as_ref());
        let zone = self.owner_zone.clone();
        let changed = triggers::update(&self.shared, &trigger.id.to_string(), |stored| {
            if let Some(name) = &name {
                stored.name = name.clone();
            }
            if let Some(action) = &action {
                stored.action = action.clone();
            }
            if let Some(schedule) = &schedule {
                stored.kind = TriggerKind::Schedule {
                    schedule: schedule.clone(),
                };
                // A changed schedule runs again, even one that ran once.
                stored.enabled = true;
            }
            if let Some(target) = &deliver_to {
                stored.deliver_to = Some(target.clone());
            }
            stored.made_by = made_by.clone();
            stored.in_zone(&zone);
            Ok(())
        });
        match changed {
            Ok(trigger) => vak_tools::ToolOutput::ok(format!(
                "changed {}",
                summary(&trigger, &self.owner_zone)
            )),
            Err(e) => refuse(Fault::Shape, e.to_string()),
        }
    }

    fn pause(
        &self,
        trigger: &Trigger,
        enabled: bool,
        ctx: &vak_tools::ToolContext,
    ) -> vak_tools::ToolOutput {
        let made_by = triggers::MadeBy::of_call(ctx.trace.as_ref());
        match triggers::update(&self.shared, &trigger.id.to_string(), |stored| {
            stored.enabled = enabled;
            stored.made_by = made_by.clone();
            Ok(())
        }) {
            Ok(trigger) => vak_tools::ToolOutput::ok(format!(
                "{} {}",
                if enabled { "resumed" } else { "paused" },
                summary(&trigger, &self.owner_zone)
            )),
            Err(e) => refuse(Fault::Tool, e.to_string()),
        }
    }
}

#[async_trait::async_trait]
impl vak_tools::Tool for AutomationsTool {
    fn name(&self) -> &str {
        "automations"
    }

    /// Its results are Vak's own records of the owner's
    /// (docs/design/08-permissions.md "Provenance").
    fn provenance(&self) -> vak_control::Provenance {
        vak_control::Provenance::PRIVATE
    }

    fn serves(&self) -> Vec<&str> {
        vec!["orchestration"]
    }

    /// Loaded on every turn: a schedule is asked for in plain conversation
    /// ("remind me", "every Monday", "keep this current"). Deferred, a
    /// replay of a captured request asking for a page kept current reached
    /// it in 0 of 16 trials; loaded, in 11 of 16 (2026-10-17).
    fn always_loaded(&self) -> bool {
        true
    }

    fn permission(&self) -> vak_permission::Declared {
        vak_permission::Declared::reach(vak_permission::Reach::Schedule)
    }

    fn description(&self) -> &str {
        "Manage the owner's automations: prompts or shell checks that run on \
         their own later, and Library pages a scheduled prompt keeps current. Use for any request to be reminded, checked in on, \
         or have something run on a schedule, and to change, pause, resume, \
         run now or remove one ('remind me on Friday at 9', 'every weekday \
         at 8 send me the headlines', 'move my morning line to 9', 'pause \
         it', 'run it now'). `add` creates one; its result comes back in \
         this conversation, or, with `as_page`, becomes the next version of \
         a Library page that stays current. For a single moment give `at`, the person's \
         local date and time ('2026-10-17 09:00'); for a repeating schedule \
         give `cron` (5 fields: min hour dom mon dow, e.g. '0 9 * * 1-5' for \
         weekdays at 9am) or `every_secs`; times are the owner's time zone \
         unless `timezone` names another. Give `prompt` (a task for you, \
         run as a fresh turn later) or `script` (a one-line shell check; \
         only its output is reported). When the day or time is ambiguous \
         and it matters, confirm before adding. `change` takes any of name, \
         prompt or script, a schedule, or deliver_to; `pause`, `resume`, \
         `run_now`, `change` and `remove` name the automation by its name \
         or id in `automation`. `list` shows each one in plain words with \
         its next and last run."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "add", "change", "pause", "resume", "run_now", "remove"],
                    "description": "What to do. `run_now` runs it once at once, paused or not, and leaves its schedule and pause as they are; `resume` turns a paused schedule back on and runs nothing now; `pause` stops its schedule until resumed"
                },
                "automation": {
                    "type": "string",
                    "description": "The automation's name or id, required for change, pause, resume, run_now and remove"
                },
                "name": {
                    "type": "string",
                    "description": "Short label: required for add; for change, its new name"
                },
                "prompt": {
                    "type": "string",
                    "description": "What you should do when it runs, in your own words (not with 'script')"
                },
                "script": {
                    "type": "string",
                    "description": "A one-line shell command to run instead of a prompt turn (not with 'prompt')"
                },
                "cron": {
                    "type": "string",
                    "description": "5-field cron expression for a repeating schedule, read in the owner's time zone unless `timezone` names another (not with 'every_secs' or 'at')"
                },
                "every_secs": {
                    "type": "integer",
                    "description": "Plain interval in seconds, from now, at least 60 (not with 'cron' or 'at')"
                },
                "timezone": {
                    "type": "string",
                    "description": "IANA time zone the cron is read in, such as America/New_York, only when it is not the owner's own"
                },
                "at": {
                    "type": "string",
                    "description": "A single future date and time as the person's local time, like '2026-10-17 09:00' (not with 'cron' or 'every_secs')"
                },
                "deliver_to": {
                    "type": "string",
                    "description": "Also send each result to '<surface>:<chat>'"
                },
                "model_pin": {
                    "type": "string",
                    "description": "Pin a prompt automation to one model id instead of the default"
                },
                "agent": {
                    "type": "string",
                    "description": "For add: a saved Agent name or id to run it as; defaults to you"
                },
                "as_page": {
                    "type": "boolean",
                    "description": "For add with a prompt: make a Library page named like the automation; each run's answer, its cards included, becomes the page's next version instead of coming back in a conversation"
                },
                "page": {
                    "type": "string",
                    "description": "For add with a prompt: the id of a Library page to keep current the same way (not with 'as_page')"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &vak_tools::ToolContext) -> vak_tools::ToolOutput {
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        let all = match triggers::list(&self.shared) {
            Ok(all) => triggers::for_workspace(all, &self.cwd),
            Err(e) => return refuse(Fault::Tool, e.to_string()),
        };
        match action {
            "list" if all.is_empty() => {
                vak_tools::ToolOutput::ok("no automations in this workspace yet".to_string())
            }
            "list" => vak_tools::ToolOutput::ok(
                serde_json::to_string_pretty(
                    &all.iter()
                        .map(|trigger| render(trigger, &self.runs, &self.owner_zone))
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_default(),
            ),
            "add" => self.add(args, ctx),
            "change" | "pause" | "resume" | "run_now" | "remove" => {
                let trigger = match Self::find(&all, args) {
                    Ok(trigger) => trigger,
                    Err((fault, text)) => return refuse(fault, text),
                };
                match action {
                    "change" => self.change(trigger, args, ctx),
                    "pause" => self.pause(trigger, false, ctx),
                    "resume" => self.pause(trigger, true, ctx),
                    "run_now" => match &self.runner {
                        None => refuse(
                            Fault::Refused,
                            "automations run only where Vakyartha's scheduler runs, and it is not running here",
                        ),
                        Some(runner) => match runner.run_now(trigger.id.to_string()).await {
                            Ok(()) => vak_tools::ToolOutput::ok(format!(
                                "started '{}'; its result comes back in its conversation",
                                trigger.name
                            )),
                            Err(reason) => refuse(Fault::Refused, reason),
                        },
                    },
                    _ if trigger.source().is_some() => refuse(
                        Fault::Refused,
                        format!(
                            "'{}' polls an intake source; remove the source instead",
                            trigger.name
                        ),
                    ),
                    _ => {
                        match triggers::delete(&self.shared, &self.runs, &trigger.id.to_string()) {
                            Ok(_) => {
                                vak_tools::ToolOutput::ok(format!("removed '{}'", trigger.name))
                            }
                            Err(e) => refuse(Fault::Tool, e.to_string()),
                        }
                    }
                }
            }
            other => refuse(
                Fault::Shape,
                format!(
                    "unknown action '{other}'; expected list, add, change, pause, resume, run_now or remove"
                ),
            ),
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
            owner_zone: "Asia/Kolkata".into(),
            runner: None,
            artifacts: crate::artifacts::Artifacts::at(
                &vak_config::scope::SharedScope::new(home),
                vak_config::paths::local_tenant_home(),
            ),
        }
    }

    fn ctx() -> vak_tools::ToolContext {
        vak_tools::ToolContext::new(std::path::PathBuf::from("/"))
    }

    /// `as_page` makes a page the automation keeps current (docs/design/98
    /// §12); `page` names only a page, and only a prompt keeps one.
    #[tokio::test]
    async fn an_automation_keeps_a_page_current() {
        vak_config::paths::isolate_home_for_tests();
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        vak_config::spaces::bind(cwd.path()).unwrap();
        let tool = tool(home.path(), cwd.path());
        let add = |extra: Value| {
            let mut args = serde_json::json!({
                "action": "add", "name": "board", "prompt": "the numbers", "cron": "0 9 * * 1",
            });
            args.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            args
        };
        let made = tool
            .execute(&add(serde_json::json!({"as_page": true})), &ctx())
            .await;
        assert!(!made.is_error(), "{}", made.content);
        let stored = triggers::list(&tool.shared).unwrap();
        let page = stored[0].page.expect("the automation names its page");
        let artifact = tool.artifacts.get(&page.to_string()).unwrap();
        assert_eq!(artifact.kind, crate::artifacts::ArtifactKind::Page);
        assert_eq!(artifact.name(), "board");
        assert!(made.content.contains(&page.to_string()), "{}", made.content);

        let again = tool
            .execute(&add(serde_json::json!({"page": page.to_string()})), &ctx())
            .await;
        assert!(!again.is_error(), "an existing page: {}", again.content);
        let file = tool
            .artifacts
            .declare(
                &stored[0].space,
                "vak",
                "notes.md",
                crate::artifacts::ArtifactKind::File,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        for refused in [
            add(serde_json::json!({"page": file.to_string()})),
            add(serde_json::json!({"page": page.to_string(), "as_page": true})),
            serde_json::json!({"action": "add", "name": "x", "script": "true", "cron": "0 9 * * 1", "as_page": true}),
        ] {
            let out = tool.execute(&refused, &ctx()).await;
            assert!(out.is_error(), "refused: {refused}");
        }
        assert_eq!(triggers::list(&tool.shared).unwrap().len(), 2);
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
        assert!(!added.is_error(), "{}", added.content);
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
        assert!(
            listed
                .content
                .contains("Weekdays at 9:00 AM, Europe/London time"),
            "{}",
            listed.content
        );
        let paused = tool
            .execute(
                &serde_json::json!({"action": "pause", "automation": "Standup"}),
                &ctx(),
            )
            .await;
        assert!(!paused.is_error(), "{}", paused.content);
        assert!(!triggers::get(&tool.shared, &id).unwrap().unwrap().enabled);
        let moved = tool
            .execute(
                &serde_json::json!({"action": "change", "automation": "standup", "cron": "30 10 * * 1-5"}),
                &ctx(),
            )
            .await;
        assert!(
            moved.content.contains("Weekdays at 10:30 AM"),
            "{}",
            moved.content
        );
        let stored = triggers::get(&tool.shared, &id).unwrap().unwrap();
        assert!(stored.enabled, "a changed schedule runs again");
        assert_eq!(
            stored.schedule(),
            Some(&Schedule::Cron {
                expr: "30 10 * * 1-5".into(),
                timezone: Some("Asia/Kolkata".into())
            }),
            "a changed cron is made in the owner's zone"
        );
        let not_here = tool
            .execute(
                &serde_json::json!({"action": "run_now", "automation": id}),
                &ctx(),
            )
            .await;
        assert!(not_here.is_error(), "{}", not_here.content);
        let removed = tool
            .execute(
                &serde_json::json!({"action": "remove", "automation": id}),
                &ctx(),
            )
            .await;
        assert!(!removed.is_error(), "{}", removed.content);
        assert!(triggers::list(&tool.shared).unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_name_two_automations_share_is_refused_with_both() {
        vak_config::paths::isolate_home_for_tests();
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        vak_config::spaces::bind(cwd.path()).unwrap();
        let tool = tool(home.path(), cwd.path());
        for _ in 0..2 {
            let added = tool
                .execute(
                    &serde_json::json!({"action": "add", "name": "line", "prompt": "p", "every_secs": 600}),
                    &ctx(),
                )
                .await;
            assert!(!added.is_error(), "{}", added.content);
        }
        let out = tool
            .execute(
                &serde_json::json!({"action": "pause", "automation": "line"}),
                &ctx(),
            )
            .await;
        assert!(out.is_error());
        for trigger in triggers::list(&tool.shared).unwrap() {
            assert!(
                out.content.contains(&trigger.id.to_string()),
                "{}",
                out.content
            );
            assert!(trigger.enabled);
        }
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
            assert!(out.is_error(), "{args}");
        }
        assert!(triggers::list(&tool.shared).unwrap().is_empty());
        let unknown = tool
            .execute(
                &serde_json::json!({"action": "remove", "automation": "trg_x"}),
                &ctx(),
            )
            .await;
        assert!(unknown.is_error());
    }
}
