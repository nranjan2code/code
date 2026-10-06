//! `vak triggers` (plan M4.3, docs/design/29-personal-os.md P2): the
//! automations of this data home, read and written through
//! `vak_core::triggers` without the server. Validation and slot math live
//! in the library; this module maps flags onto a `Trigger` and renders the
//! table, with each one's last run read from the run records.

use std::path::Path;

use vak_core::Core;
use vak_core::triggers::{self, OnCrash, Schedule, Trigger, TriggerAction, TriggerKind};

pub fn run_triggers(cwd: std::path::PathBuf, action: crate::cli::TriggersAction) -> i32 {
    let core = match Core::new(cwd.clone()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let shared = core.shared_scope();
    match action {
        crate::cli::TriggersAction::List => match triggers::list(&shared) {
            Ok(all) => {
                list(&all, &core.runs());
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        crate::cli::TriggersAction::Add(add) => {
            let crate::cli::TriggerAdd {
                name,
                prompt,
                script,
                every,
                cron,
                timezone,
                cwd: trigger_cwd,
                deliver,
                model,
                preset,
            } = *add;
            let dir = trigger_cwd.unwrap_or_else(|| cwd.clone());
            let built = match preset.as_deref() {
                None => build_trigger(
                    name.as_deref().unwrap_or_default(),
                    prompt.as_deref(),
                    script.as_deref(),
                    every,
                    cron.as_deref(),
                    timezone.as_deref(),
                    &dir,
                    deliver.as_deref(),
                    model.as_deref(),
                ),
                Some(preset_name) => expand_preset(preset_name, deliver.as_deref()).and_then(|x| {
                    // clap rejects the content and schedule flags with
                    // --preset; an explicit --name may rename it.
                    let trigger_name = name
                        .as_deref()
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .unwrap_or(x.default_name);
                    build_trigger(
                        trigger_name,
                        None,
                        Some(&x.script),
                        None,
                        Some(x.schedule),
                        timezone.as_deref(),
                        &dir,
                        Some(&x.deliver_to),
                        None,
                    )
                }),
            };
            let trigger = match built {
                Ok(trigger) => trigger,
                Err(msg) => {
                    eprintln!("error: {msg}");
                    return 2;
                }
            };
            match triggers::create(&shared, &trigger) {
                Ok(()) => {
                    println!("added automation {} · {}", trigger.id, detail(&trigger));
                    println!("      next run {}", next_run(&trigger, chrono::Utc::now()));
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    2
                }
            }
        }
        crate::cli::TriggersAction::Remove { id } => match triggers::delete(&shared, &id) {
            Ok(true) => {
                println!("removed automation {id}");
                0
            }
            Ok(false) => {
                eprintln!("error: no automation '{id}'");
                2
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        crate::cli::TriggersAction::Enable { id } => set_enabled(&shared, &id, true),
        crate::cli::TriggersAction::Disable { id } => set_enabled(&shared, &id, false),
    }
}

fn set_enabled(shared: &vak_config::scope::SharedScope, id: &str, enable: bool) -> i32 {
    match triggers::update(shared, id, |trigger| {
        trigger.enabled = enable;
        Ok(())
    }) {
        Ok(_) => {
            println!(
                "{} automation {id}",
                if enable { "enabled" } else { "disabled" }
            );
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

/// Flag mapping and schedule exclusivity, kept pure for tests (binding the
/// folder aside).
#[allow(clippy::too_many_arguments)]
pub fn build_trigger(
    name: &str,
    prompt: Option<&str>,
    script: Option<&str>,
    every: Option<u64>,
    cron: Option<&str>,
    timezone: Option<&str>,
    cwd: &Path,
    deliver: Option<&str>,
    model_pin: Option<&str>,
) -> Result<Trigger, String> {
    let trimmed = |value: Option<&str>| {
        value
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    if name.trim().is_empty() {
        return Err("--name must not be empty".into());
    }
    if every.is_some() && cron.is_some() {
        return Err("pass --every or --cron, not both".into());
    }
    let action = match (trimmed(prompt), trimmed(script)) {
        (Some(text), None) => TriggerAction::Prompt {
            text,
            model_pin: trimmed(model_pin),
        },
        (None, Some(command)) => TriggerAction::Script { command },
        _ => return Err("exactly one of --prompt or --script is required".into()),
    };
    let now = chrono::Utc::now();
    let schedule = match trimmed(cron) {
        Some(expr) => Schedule::Cron {
            expr,
            timezone: trimmed(timezone),
        },
        None => Schedule::Interval {
            every_secs: every.unwrap_or(3600),
            anchor: now,
        },
    };
    let trigger = Trigger {
        id: vak_session::ids::TriggerId::new(),
        name: name.trim().to_string(),
        agent: "vak".into(),
        agent_revision: None,
        space: vak_config::spaces::bind(cwd)?,
        enabled: true,
        kind: TriggerKind::Schedule { schedule },
        action,
        deliver_to: trimmed(deliver),
        on_crash: OnCrash::Skip,
        scope: None,
        created_at: now,
        created_by: Some(vak_session::trace::local::local_owner()),
    };
    trigger.validate().map_err(|e| e.to_string())?;
    Ok(trigger)
}

/// A built-in `triggers add --preset` expansion (docs/design/29-personal-os.md
/// P2/P3). Script presets only: each runs this binary as a shell one-liner.
struct Preset {
    name: &'static str,
    /// Argument vector appended after the current executable.
    args: &'static [&'static str],
    schedule: &'static str,
    deliver_default: &'static str,
}

const PRESETS: &[Preset] = &[Preset {
    name: "weekly-digest",
    args: &["digest", "--days", "7"],
    schedule: "0 9 * * 1",
    deliver_default: "log:vak",
}];

#[derive(Debug)]
pub(crate) struct PresetExpansion {
    pub default_name: &'static str,
    pub script: String,
    pub schedule: &'static str,
    pub deliver_to: String,
}

/// Expands a preset, resolving the running binary so the automation
/// invokes this exact build even after an upgrade moves it.
pub(crate) fn expand_preset(
    preset: &str,
    deliver: Option<&str>,
) -> Result<PresetExpansion, String> {
    let exe = std::env::current_exe().map_err(|e| format!("resolve current executable: {e}"))?;
    expand_preset_with(preset, &exe, deliver)
}

fn expand_preset_with(
    preset: &str,
    exe: &Path,
    deliver: Option<&str>,
) -> Result<PresetExpansion, String> {
    let known = PRESETS.iter().find(|p| p.name == preset).ok_or_else(|| {
        format!(
            "unknown preset '{preset}' (known: {})",
            PRESETS
                .iter()
                .map(|p| p.name)
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    Ok(PresetExpansion {
        default_name: known.name,
        script: format!("{} {}", shell_quote(exe), known.args.join(" ")),
        schedule: known.schedule,
        deliver_to: deliver
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| known.deliver_default.to_string()),
    })
}

/// POSIX single-quote wrapper; embedded quotes become the `'\''` idiom.
fn shell_quote(path: &Path) -> String {
    let raw = path.to_string_lossy();
    format!("'{}'", raw.replace('\'', "'\\''"))
}

fn schedule_text(trigger: &Trigger) -> String {
    match trigger.schedule() {
        Some(Schedule::Cron { expr, timezone }) => match timezone {
            Some(zone) => format!("{expr} ({zone})"),
            None => expr.clone(),
        },
        Some(Schedule::Interval { every_secs, .. }) => format!("every {every_secs}s"),
        Some(Schedule::Once { at }) => format!("once at {}", at.to_rfc3339()),
        None => "manual".into(),
    }
}

fn list(all: &[Trigger], runs: &vak_session::runs::Runs) {
    if all.is_empty() {
        println!("no automations");
        return;
    }
    let width = all.iter().map(|t| t.name.len()).max().unwrap_or(0);
    println!(
        "  {:<width$}  {:<3}  {:<18}  {:<22}  last run",
        "name",
        "on",
        "schedule",
        "next run",
        width = width
    );
    let now = chrono::Utc::now();
    for trigger in all {
        let last = triggers::last_run(runs, &trigger.id)
            .ok()
            .flatten()
            .map(|run| {
                let status = serde_json::to_value(run.status)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default();
                format!(
                    "{status} {}",
                    run.opened_at
                        .with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M")
                )
            })
            .unwrap_or_else(|| "—".into());
        println!(
            "  {:<width$}  {:<3}  {:<18}  {:<22}  {last}",
            trigger.name,
            if trigger.enabled { "yes" } else { "no" },
            schedule_text(trigger),
            next_run(trigger, now),
            width = width
        );
        println!("      id {} · {}", trigger.id, detail(trigger));
    }
}

fn next_run(trigger: &Trigger, now: chrono::DateTime<chrono::Utc>) -> String {
    if !trigger.enabled {
        return "(disabled)".into();
    }
    match trigger.next_slot_after(now) {
        Some(next) => next
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M (%a)")
            .to_string(),
        None => "—".into(),
    }
}

/// One line of what it does, shown after every add and in the list.
fn detail(trigger: &Trigger) -> String {
    let mut parts = vec![match &trigger.action {
        TriggerAction::Script { command } => format!("script: {command}"),
        TriggerAction::SourcePoll { source } => format!("polls source {source}"),
        TriggerAction::Prompt { text, model_pin } => {
            let mut line = format!("prompt: {}", text.lines().next().unwrap_or_default());
            if let Some(model) = model_pin {
                line.push_str(&format!(" · model {model}"));
            }
            line
        }
    }];
    if let Some(d) = &trigger.deliver_to {
        parts.push(format!("deliver {d}"));
    }
    match trigger.workspace() {
        Some(folder) => parts.push(format!("in {}", folder.display())),
        None => parts.push(format!(
            "space {} (no folder on this machine)",
            trigger.space
        )),
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn folder() -> tempfile::TempDir {
        vak_config::paths::isolate_home_for_tests();
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn build_requires_prompt_xor_script_and_one_schedule() {
        let dir = folder();
        let build = |prompt, script, every, cron| {
            build_trigger(
                "t",
                prompt,
                script,
                every,
                cron,
                None,
                dir.path(),
                None,
                None,
            )
        };
        assert!(build(None, None, None, None).is_err());
        assert!(build(Some("p"), Some("true"), None, None).is_err());
        assert!(build(Some("p"), None, Some(600), Some("0 9 * * *")).is_err());
        assert!(build(Some("p"), None, None, Some("99 * * * *")).is_err());
        let interval = build(Some("p"), None, None, None).unwrap();
        assert!(matches!(
            interval.schedule(),
            Some(Schedule::Interval {
                every_secs: 3600,
                ..
            })
        ));
        let cron = build(None, Some("df -h"), None, Some("*/15 * * * *")).unwrap();
        assert_eq!(cron.script(), Some("df -h"));
        assert!(
            build_trigger(
                "  ",
                Some("p"),
                None,
                None,
                None,
                None,
                dir.path(),
                None,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn a_model_pin_rides_on_the_prompt() {
        let dir = folder();
        let pinned = build_trigger(
            "t",
            Some("p"),
            None,
            Some(600),
            None,
            None,
            dir.path(),
            Some("telegram:1"),
            Some("ollama/gemma"),
        )
        .unwrap();
        assert_eq!(pinned.model_pin(), Some("ollama/gemma"));
        assert_eq!(pinned.deliver_to.as_deref(), Some("telegram:1"));
    }

    #[test]
    fn weekly_digest_preset_expands_and_lands_on_monday() {
        let dir = folder();
        let exe = Path::new("/opt/it's here/vak");
        let x = expand_preset_with("weekly-digest", exe, None).unwrap();
        assert_eq!(x.script, "'/opt/it'\\''s here/vak' digest --days 7");
        assert_eq!(x.deliver_to, "log:vak");
        let overridden = expand_preset_with("weekly-digest", exe, Some("telegram:9")).unwrap();
        assert_eq!(overridden.deliver_to, "telegram:9");
        assert!(expand_preset_with("nope", exe, None).is_err());
        let trigger = build_trigger(
            x.default_name,
            None,
            Some(&x.script),
            None,
            Some(x.schedule),
            Some("UTC"),
            dir.path(),
            Some(&x.deliver_to),
            None,
        )
        .unwrap();
        let next = trigger.next_slot_after(chrono::Utc::now()).unwrap();
        assert_eq!(chrono::Datelike::weekday(&next), chrono::Weekday::Mon);
    }
}
