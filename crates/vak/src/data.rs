//! `vak data` (plan M7a-c): what the data home holds and what its
//! retention would do, read directly like `vak runs`. Observe-only: every
//! verb here reads, and the plan it prints is not carried out.

use std::path::PathBuf;

use vak_core::Core;

fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn name<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(|name| name.replace('_', " ")))
        .unwrap_or_default()
}

pub(crate) fn run_data(cwd: PathBuf, action: Option<crate::cli::DataAction>) -> i32 {
    let core = match Core::new(cwd) {
        Ok(core) => core,
        Err(error) => {
            eprintln!("error: {error}");
            return 2;
        }
    };
    match action.unwrap_or(crate::cli::DataAction::Status) {
        crate::cli::DataAction::Status => {
            let status = core.data_status();
            println!(
                "retention     {}",
                if status.mode == "commit" {
                    "removing what is due (for the kinds listed as carried out)"
                } else {
                    "observing only: the plan is shown and nothing is removed"
                }
            );
            println!("label         {}", status.label.name);
            println!(
                "held          {} in {} files",
                size(status.bytes),
                status.files
            );
            println!(
                "due now       {} items, {}",
                status.due,
                size(status.reclaimable_bytes)
            );
            println!(
                "kept back     {} items (in use, or kept by a person)",
                status.guarded
            );
            let list = |classes: &[vak_lifecycle::DataClass]| {
                classes.iter().map(name).collect::<Vec<_>>().join(", ")
            };
            println!("watched       {}", list(&status.observed));
            println!("carried out   {}", list(&status.committed));
            println!("not watched   {}", list(&status.unobserved));
            0
        }
        crate::cli::DataAction::Usage => {
            let usage = core.data_usage();
            println!(
                "{:<9} {:<12} {:<22} {:>8} {:>10}",
                "root", "class", "owner", "files", "size"
            );
            for row in &usage.rows {
                println!(
                    "{:<9} {:<12} {:<22} {:>8} {:>10}",
                    row.root,
                    row.class,
                    row.owner,
                    row.files,
                    size(row.bytes)
                );
            }
            println!(
                "{:<9} {:<12} {:<22} {:>8} {:>10}",
                "total",
                "",
                "",
                usage.files,
                size(usage.bytes)
            );
            0
        }
        crate::cli::DataAction::Gc { dry_run } => {
            let allowed = core.config().lifecycle.commit;
            let tick = core.lifecycle_tick(allowed && !dry_run);
            for row in &tick.committed {
                println!(
                    "removed {:<16} {:>10}  {}",
                    name(&row.class),
                    size(row.bytes),
                    row.item
                );
            }
            for row in &tick.failed {
                println!(
                    "failed  {:<16} {}  ({})",
                    name(&row.class),
                    row.item,
                    row.error_kind.as_deref().unwrap_or("unknown")
                );
            }
            if tick.mode == "commit" {
                println!(
                    "removed {} items, {}; {} due items are of kinds not carried out yet",
                    tick.committed.len(),
                    size(tick.reclaimed_bytes),
                    tick.left
                );
            } else {
                println!(
                    "{} items are due, {}; nothing was removed{}",
                    tick.plan.actions.len(),
                    size(tick.plan.reclaimable_bytes),
                    if allowed || dry_run {
                        ""
                    } else {
                        " (retention is observing; set [lifecycle] mode = \"commit\" to act)"
                    }
                );
            }
            i32::from(!tick.failed.is_empty())
        }
        crate::cli::DataAction::Plan { json } => {
            let plan = core.lifecycle_plan();
            if json {
                return match serde_json::to_string_pretty(&plan) {
                    Ok(text) => {
                        println!("{text}");
                        0
                    }
                    Err(error) => {
                        eprintln!("error: {error}");
                        1
                    }
                };
            }
            if plan.actions.is_empty() {
                println!("nothing is due");
            }
            for action in &plan.actions {
                println!(
                    "{:<7} {:<16} {:>10}  due {}  {}",
                    name(&action.does),
                    name(&action.class),
                    size(action.bytes),
                    action.due.format("%Y-%m-%d"),
                    action.item
                );
            }
            for kept in &plan.guarded {
                println!(
                    "{:<7} {:<16} {:>10}  {}  {}",
                    "keep",
                    name(&kept.class),
                    "",
                    name(&kept.guard),
                    kept.item
                );
            }
            if !plan.unobserved.is_empty() {
                let classes: Vec<String> = plan.unobserved.iter().map(name).collect();
                println!("not watched yet: {}", classes.join(", "));
            }
            println!("this is a dry run: nothing was removed");
            0
        }
    }
}
