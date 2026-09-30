//! `vak tasks` (docs/design/29-personal-os.md P2): CRUD over
//! `vak_core::tasks::TaskStore` (tasks.json in the sessions home) without the
//! server. Validation and cron math live in the library; this module only
//! maps flags onto `TaskDef` and renders the table.

use std::path::{Path, PathBuf};

use vak_core::Core;
use vak_core::tasks::{TaskDef, TaskStore, cron_next_after};

pub fn run_tasks(cwd: PathBuf, action: crate::cli::TasksAction) -> i32 {
    let core = match Core::new(cwd.clone()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let home = core.sessions_home();
    let mut store = match TaskStore::load(&home) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    match action {
        crate::cli::TasksAction::List => {
            list(&store);
            0
        }
        crate::cli::TasksAction::Add {
            name,
            prompt,
            script,
            every,
            cron,
            cwd: task_cwd,
            deliver,
            model,
            preset,
        } => {
            let task_dir = task_cwd.unwrap_or_else(|| cwd.clone());
            let built = match preset.as_deref() {
                None => build_task_def(
                    name.as_deref().unwrap_or_default(),
                    prompt.as_deref(),
                    script.as_deref(),
                    every,
                    cron.as_deref(),
                    &task_dir,
                    deliver.as_deref(),
                    model.as_deref(),
                ),
                Some(preset_name) => expand_preset(preset_name, deliver.as_deref()).and_then(|x| {
                    // clap rejects the content/schedule flags with
                    // --preset; an explicit --name may rename the task.
                    let task_name = name
                        .as_deref()
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .unwrap_or(x.default_name);
                    build_task_def(
                        task_name,
                        None,
                        Some(&x.script),
                        None,
                        Some(x.schedule),
                        &task_dir,
                        Some(&x.deliver_to),
                        model.as_deref(),
                    )
                }),
            };
            match built {
                Ok(task) => {
                    if let Err(e) = task.validate() {
                        eprintln!("error: {e}");
                        return 2;
                    }
                    let id = task.id.clone();
                    store.put(task.clone());
                    save_and_report(&mut store, || {
                        println!("added task {id} · {}", add_detail(&task));
                        println!(
                            "      next fire {}",
                            next_fire_preview(&task, chrono::Local::now())
                        );
                    })
                }
                Err(msg) => {
                    eprintln!("error: {msg}");
                    2
                }
            }
        }
        crate::cli::TasksAction::Remove { id } => {
            if store.remove(&id) {
                save_and_report(&mut store, || println!("removed task {id}"))
            } else {
                eprintln!("error: no task '{id}'");
                2
            }
        }
        crate::cli::TasksAction::Enable { id } => set_enabled(&mut store, &id, true),
        crate::cli::TasksAction::Disable { id } => set_enabled(&mut store, &id, false),
    }
}

fn set_enabled(store: &mut TaskStore, id: &str, enable: bool) -> i32 {
    match store.get(id).cloned() {
        Some(mut task) => {
            task.enabled = enable;
            store.put(task);
            save_and_report(store, || {
                println!("{} task {id}", if enable { "enabled" } else { "disabled" })
            })
        }
        None => {
            eprintln!("error: no task '{id}'");
            2
        }
    }
}

fn save_and_report(store: &mut TaskStore, report: impl Fn()) -> i32 {
    match store.save() {
        Ok(()) => {
            report();
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// Flag mapping + schedule exclusivity, kept pure for tests.
#[allow(clippy::too_many_arguments)]
pub fn build_task_def(
    name: &str,
    prompt: Option<&str>,
    script: Option<&str>,
    every: Option<u64>,
    cron: Option<&str>,
    cwd: &Path,
    deliver: Option<&str>,
    model_pin: Option<&str>,
) -> Result<TaskDef, String> {
    if name.trim().is_empty() {
        return Err("--name must not be empty".into());
    }
    if every.is_some() && cron.is_some() {
        return Err("pass --every or --cron, not both".into());
    }
    let (prompt, script) = match (prompt.map(str::trim), script.map(str::trim)) {
        (Some(p), None) if !p.is_empty() => (p.to_string(), None),
        (None, Some(s)) if !s.is_empty() => (String::new(), Some(s.to_string())),
        (None, None) => {
            return Err("exactly one of --prompt or --script is required".into());
        }
        _ => return Err("exactly one of --prompt or --script is required".into()),
    };
    Ok(TaskDef {
        // Nanosecond timestamp ids sort like the server's uuid-v7 ids.
        id: format!("cli-{}", timestamp_id()),
        name: name.trim().to_string(),
        interval_secs: every.unwrap_or(3600),
        enabled: true,
        cwd: cwd.to_path_buf(),
        created_at: chrono::Utc::now(),
        last_run_at: None,
        last_session_id: None,
        last_summary: None,
        last_result_id: None,
        last_run_status: None,
        mail_calendar_last_check_at: None,
        last_delivery_state: None,
        last_wt: None,
        deliver_to: deliver
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_string),
        schedule: cron
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_string),
        timezone: None,
        due_at: None,
        script,
        model_pin: model_pin
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_string),
        agent_id: None,
        agent_revision: None,
        mail_calendar_scope: None,
        prompt,
    })
}

fn timestamp_id() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
}

/// A built-in `tasks add --preset` expansion (docs/design/29-personal-os.md
/// P2/P3). Script presets only: each runs this binary as a shell one-liner,
/// so adding a preset must never require new prompt semantics.
struct TaskPreset {
    name: &'static str,
    /// Argument vector appended after the current executable.
    args: &'static [&'static str],
    schedule: &'static str,
    deliver_default: &'static str,
}

const TASK_PRESETS: &[TaskPreset] = &[TaskPreset {
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

/// Expand a preset, resolving the running binary so the task invokes this
/// exact build even after upgrades move it.
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
    let known = TASK_PRESETS
        .iter()
        .find(|p| p.name == preset)
        .ok_or_else(|| {
            format!(
                "unknown preset '{preset}' (known: {})",
                TASK_PRESETS
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

fn list(store: &TaskStore) {
    let tasks = store.all();
    if tasks.is_empty() {
        println!("no scheduled tasks");
        return;
    }
    let width = tasks.iter().map(|t| t.name.len()).max().unwrap_or(0);
    let header = format!(
        "  {:<width$}  {:<3}  {:<12}  {}",
        "name",
        "on",
        "schedule",
        "next fire",
        width = width
    );
    println!("{header}");
    let now = chrono::Local::now();
    for t in &tasks {
        let schedule = match (&t.schedule, t.interval_secs) {
            (Some(expr), _) => expr.clone(),
            (None, secs) => format!("every {secs}s"),
        };
        let next = next_fire_preview(t, now);
        println!(
            "  {:<width$}  {:<3}  {:<12}  {}",
            t.name,
            if t.enabled { "yes" } else { "no" },
            schedule,
            next,
            width = width
        );
        println!("      id {} · {}", t.id, kind_line(t));
    }
}

fn next_fire_preview(task: &TaskDef, now: chrono::DateTime<chrono::Local>) -> String {
    let Some(expr) = &task.schedule else {
        return "—".into();
    };
    if !task.enabled {
        return "(disabled)".into();
    }
    match cron_next_after(expr, now) {
        Ok(next) => next.format("%Y-%m-%d %H:%M (%a)").to_string(),
        Err(reason) => format!("invalid: {reason}"),
    }
}

/// One-line expanded definition shown after every successful add.
fn add_detail(t: &TaskDef) -> String {
    let schedule = match &t.schedule {
        Some(expr) => format!("schedule {expr}"),
        None => format!("every {}s", t.interval_secs),
    };
    format!("{schedule} · {}", kind_line(t))
}

fn kind_line(t: &TaskDef) -> String {
    let mut parts = vec![if t.script.is_some() {
        format!("script: {}", t.script.as_deref().unwrap_or(""))
    } else {
        format!("prompt: {}", first_line(&t.prompt))
    }];
    if let Some(m) = &t.model_pin {
        parts.push(format!("model {m}"));
    }
    if let Some(d) = &t.deliver_to {
        parts.push(format!("deliver {d}"));
    }
    parts.push(format!("cwd {}", t.cwd.display()));
    parts.join(" · ")
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn add_args() -> (&'static str, Option<&'static str>, Option<&'static str>) {
        ("nightly", Some("tidy the repo"), None)
    }

    #[test]
    fn build_requires_prompt_xor_script() {
        let (name, prompt, _) = add_args();
        assert!(
            build_task_def(name, prompt, None, None, None, Path::new("/w"), None, None).is_ok()
        );
        assert!(
            build_task_def(
                name,
                None,
                Some("echo tick"),
                None,
                None,
                Path::new("/w"),
                None,
                None
            )
            .is_ok()
        );
        let both = build_task_def(
            name,
            prompt,
            Some("x"),
            None,
            None,
            Path::new("/w"),
            None,
            None,
        );
        assert_eq!(
            both.unwrap_err(),
            "exactly one of --prompt or --script is required"
        );
        let neither = build_task_def(name, None, None, None, None, Path::new("/w"), None, None);
        assert_eq!(
            neither.unwrap_err(),
            "exactly one of --prompt or --script is required"
        );
    }

    #[test]
    fn build_rejects_every_and_cron_together() {
        let (name, prompt, _) = add_args();
        let err = build_task_def(
            name,
            prompt,
            None,
            Some(60),
            Some("* * * * *"),
            Path::new("/w"),
            None,
            None,
        )
        .unwrap_err();
        assert!(err.contains("--every or --cron"), "{err}");
    }

    #[test]
    fn build_defaults_interval_and_keeps_pins() {
        let task = build_task_def(
            "nightly",
            None,
            Some("curl -sf http://x/health"),
            None,
            None,
            Path::new("/w"),
            Some("telegram:42"),
            Some(" haiku-fast "),
        )
        .unwrap();
        assert_eq!(task.interval_secs, 3600);
        assert_eq!(task.script.as_deref(), Some("curl -sf http://x/health"));
        assert_eq!(task.deliver_to.as_deref(), Some("telegram:42"));
        assert_eq!(task.model_pin.as_deref(), Some("haiku-fast"));
        assert!(task.schedule.is_none());
        assert!(task.enabled);
        assert!(task.validate().is_ok());

        let with_every = build_task_def(
            "nightly",
            None,
            Some("x"),
            Some(120),
            None,
            Path::new("/w"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(with_every.interval_secs, 120);

        let with_cron = build_task_def(
            "nightly",
            Some("tidy"),
            None,
            None,
            Some("0 7 * * 1-5"),
            Path::new("/w"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(with_cron.schedule.as_deref(), Some("0 7 * * 1-5"));
    }

    #[test]
    fn empty_name_rejected() {
        assert!(
            build_task_def(
                "  ",
                Some("p"),
                None,
                None,
                None,
                Path::new("/w"),
                None,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn bad_cron_surfaces_through_validate_not_build() {
        let (name, _, _) = add_args();
        let task = build_task_def(
            name,
            None,
            Some("x"),
            None,
            Some("99 * * * *"),
            Path::new("/w"),
            None,
            None,
        )
        .unwrap();
        assert!(matches!(
            task.validate(),
            Err(vak_core::tasks::TaskError::BadSchedule { .. })
        ));
    }

    #[test]
    fn next_fire_preview_states() {
        let mut t = base_task();
        t.schedule = Some("0 7 * * 1-5".into());
        t.enabled = false;
        assert_eq!(next_fire_preview(&t, chrono::Local::now()), "(disabled)");

        t.enabled = true;
        let fired = next_fire_preview(&t, chrono::Local::now());
        assert!(!fired.starts_with("invalid"), "{fired}");

        t.schedule = Some("0 0 31 2 *".into());
        assert!(next_fire_preview(&t, chrono::Local::now()).starts_with("invalid"));

        t.schedule = None;
        assert_eq!(next_fire_preview(&t, chrono::Local::now()), "—");
    }

    fn base_task() -> TaskDef {
        build_task_def(
            "watchdog",
            None,
            Some("true"),
            None,
            None,
            Path::new("/w"),
            None,
            None,
        )
        .unwrap()
    }

    #[test]
    fn list_renders_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = TaskStore::load(dir.path()).unwrap();
        store.put(base_task());
        let mut scheduled = base_task();
        scheduled.name = "morning brief".into();
        scheduled.prompt = "brief me".into();
        scheduled.script = None;
        scheduled.schedule = Some("0 7 * * 1-5".into());
        assert!(scheduled.validate().is_ok());
        store.put(scheduled);
        list(&store);
        let empty = TaskStore::load(dir.path()).unwrap();
        list(&empty);
    }

    fn weekly_digest_task(exe: &Path) -> TaskDef {
        let x = expand_preset_with("weekly-digest", exe, None).unwrap();
        build_task_def(
            x.default_name,
            None,
            Some(&x.script),
            None,
            Some(x.schedule),
            Path::new("/w"),
            Some(&x.deliver_to),
            None,
        )
        .unwrap()
    }

    #[test]
    fn weekly_digest_preset_expands_fully() {
        let task = weekly_digest_task(Path::new("/opt/bin/vak"));
        assert_eq!(task.name, "weekly-digest");
        assert_eq!(task.schedule.as_deref(), Some("0 9 * * 1"));
        assert_eq!(task.deliver_to.as_deref(), Some("log:vak"));
        assert_eq!(
            task.script.as_deref(),
            Some("'/opt/bin/vak' digest --days 7")
        );
        assert!(task.prompt.is_empty());
        assert!(task.model_pin.is_none());
        assert!(task.validate().is_ok());
    }

    #[test]
    fn preset_deliver_override_wins_over_default() {
        let x =
            expand_preset_with("weekly-digest", Path::new("/v"), Some(" telegram:42 ")).unwrap();
        assert_eq!(x.deliver_to, "telegram:42");
    }

    #[test]
    fn unknown_preset_lists_known_names() {
        let err = expand_preset_with("inbox-zero", Path::new("/v"), None).unwrap_err();
        assert!(err.contains("unknown preset 'inbox-zero'"), "{err}");
        assert!(err.contains("weekly-digest"), "{err}");
    }

    #[test]
    fn exe_quoting_survives_spaces_and_quotes() {
        assert_eq!(shell_quote(Path::new("/app dir/vak")), "'/app dir/vak'");
        assert_eq!(shell_quote(Path::new("/o'mal/vak")), "'/o'\\''mal/vak'");
    }

    #[test]
    fn expanded_preset_roundtrips_through_store() {
        let dir = tempfile::tempdir().unwrap();
        let mut task = weekly_digest_task(Path::new("/opt/bin/vak"));
        task.id = "preset-rt".into();
        let mut store = TaskStore::load(dir.path()).unwrap();
        store.put(task.clone());
        store.save().unwrap();
        let reloaded = TaskStore::load(dir.path())
            .unwrap()
            .get("preset-rt")
            .cloned();
        let same = reloaded.as_ref().is_some_and(|t| {
            t.name == task.name
                && t.script == task.script
                && t.schedule == task.schedule
                && t.deliver_to == task.deliver_to
                && t.prompt == task.prompt
                && t.model_pin == task.model_pin
                && t.enabled == task.enabled
        });
        assert!(same, "roundtrip lost fields: {reloaded:?}");
    }

    #[test]
    fn weekly_digest_next_fire_lands_on_monday() {
        let task = weekly_digest_task(Path::new("/opt/bin/vak"));
        let preview = next_fire_preview(&task, chrono::Local::now());
        assert!(preview.ends_with("(Mon)"), "{preview}");
    }

    #[test]
    fn add_detail_includes_schedule_script_and_delivery() {
        let task = weekly_digest_task(Path::new("/opt/bin/vak"));
        let detail = add_detail(&task);
        assert!(detail.contains("schedule 0 9 * * 1"), "{detail}");
        assert!(detail.contains("digest --days 7"), "{detail}");
        assert!(detail.contains("deliver log:vak"), "{detail}");
    }
}
