//! `vakcoder tasks` (docs/design/29-personal-os.md P2): CRUD over
//! `vak_core::tasks::TaskStore` (~/.vakcoder/tasks.json) without the
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
        } => {
            let task_dir = task_cwd.unwrap_or_else(|| cwd.clone());
            match build_task_def(
                &name,
                prompt.as_deref(),
                script.as_deref(),
                every,
                cron.as_deref(),
                &task_dir,
                deliver.as_deref(),
                model.as_deref(),
            ) {
                Ok(task) => {
                    if let Err(e) = task.validate() {
                        eprintln!("error: {e}");
                        return 2;
                    }
                    let id = task.id.clone();
                    store.put(task);
                    save_and_report(&mut store, || println!("added task {id}"))
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
        last_wt: None,
        deliver_to: deliver
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_string),
        schedule: cron
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_string),
        script,
        model_pin: model_pin
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_string),
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
}
