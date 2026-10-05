//! `vak runs` (plan M4.2): the run records read directly, the way `inbox`
//! and `tasks` read their stores. A record that cannot be read is an
//! error, never skipped.

use std::path::PathBuf;

use vak_core::Core;
use vak_session::runs::{RunRecord, RunStatus};

pub(crate) fn run_runs(cwd: PathBuf, action: Option<crate::cli::RunsAction>) -> i32 {
    let core = match Core::new(cwd) {
        Ok(core) => core,
        Err(error) => {
            eprintln!("error: {error}");
            return 2;
        }
    };
    let runs = match core.runs().list() {
        Ok(runs) => runs,
        Err(error) => {
            eprintln!("error: {error}");
            return 1;
        }
    };
    match action.unwrap_or(crate::cli::RunsAction::List {
        status: None,
        limit: 50,
    }) {
        crate::cli::RunsAction::List { status, limit } => {
            let status = match status.as_deref().map(parse_status).transpose() {
                Ok(status) => status,
                Err(error) => {
                    eprintln!("error: {error}");
                    return 2;
                }
            };
            let rows: Vec<_> = runs
                .iter()
                .filter(|run| status.is_none_or(|status| run.status == status))
                .take(limit.max(1))
                .collect();
            if rows.is_empty() {
                println!("no runs");
            }
            for run in rows {
                println!("{}", row(run));
            }
            0
        }
        crate::cli::RunsAction::Show { id } => {
            match runs.iter().find(|run| run.id.to_string() == id) {
                Some(run) => match serde_json::to_string_pretty(run) {
                    Ok(json) => {
                        println!("{json}");
                        0
                    }
                    Err(error) => {
                        eprintln!("error: {error}");
                        1
                    }
                },
                None => {
                    eprintln!("error: no run {id}");
                    1
                }
            }
        }
    }
}

fn parse_status(text: &str) -> Result<RunStatus, String> {
    serde_json::from_value(serde_json::Value::String(text.to_ascii_lowercase()))
        .map_err(|_| format!("unknown status {text}"))
}

fn cause(run: &RunRecord) -> String {
    use vak_session::runs::RunWork;
    use vak_session::trace::Cause;
    match &run.work {
        Some(RunWork::Flow { name }) => return format!("flow:{name}"),
        Some(RunWork::Plan) => return "plan".into(),
        None => {}
    }
    match run.trace.as_ref().map(|trace| &trace.cause) {
        Some(Cause::User { .. }) => "user",
        Some(Cause::Channel { .. }) => "channel",
        Some(Cause::Schedule { .. }) => "schedule",
        Some(Cause::Delegation { .. }) => "delegation",
        Some(Cause::Revision { .. }) => "revision",
        Some(Cause::Trigger { .. }) => "trigger",
        Some(Cause::Heartbeat) => "heartbeat",
        Some(Cause::System { .. }) => "system",
        None => "-",
    }
    .into()
}

fn row(run: &RunRecord) -> String {
    let status = serde_json::to_value(run.status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    format!(
        "{}  {:<10} {:<16} {}  {}{}",
        run.id,
        status,
        cause(run),
        run.opened_at.format("%Y-%m-%d %H:%M:%S"),
        run.agent().unwrap_or_else(|| "-".into()),
        run.reason
            .as_deref()
            .map(|reason| format!("  ({reason})"))
            .unwrap_or_default()
    )
}
