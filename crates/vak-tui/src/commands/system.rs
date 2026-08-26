//! System commands: doctor, services, shell passthrough, export.

use crate::data::ClientData;
use crate::render::Screen;
use crate::state::ModalView;
use crate::theme::Theme;

pub async fn run_doctor(data: &ClientData) -> ModalView {
    let mut rows = Vec::new();
    match data.doctor().await {
        Ok(report) => {
            let failures = report.get("failures").and_then(|f| f.as_u64()).unwrap_or(0);
            if let Some(checks) = report.get("checks").and_then(|c| c.as_array()) {
                for check in checks {
                    let label = str_field(check, &["label", "name"], "check");
                    let ok = check.get("ok").and_then(|o| o.as_bool()).unwrap_or(true);
                    let detail = str_field(check, &["detail", "value", "message"], "");
                    rows.push(report_line(&label, &detail, ok));
                }
            }
            if let Some(facts) = report.get("facts").and_then(|f| f.as_array()) {
                for fact in facts {
                    match fact.as_str() {
                        Some(text) => rows.push(format!("· {text}")),
                        None => {
                            let label = str_field(fact, &["label", "name", "key"], "");
                            let value = str_field(fact, &["value", "detail", "text"], "");
                            if label.is_empty() && value.is_empty() {
                                rows.push(format!("· {fact}"));
                            } else if value.is_empty() {
                                rows.push(format!("· {label}"));
                            } else {
                                rows.push(format!("· {label}: {value}"));
                            }
                        }
                    }
                }
            }
            if let Some(ladder) = report.get("ladder").filter(|l| !l.is_null()) {
                let legs = ladder
                    .get("legs")
                    .and_then(|l| l.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                match ladder.get("rendered").and_then(|r| r.as_str()) {
                    Some(rendered) => {
                        rows.push(format!("· route ladder (frozen at admission): {rendered}"));
                    }
                    None => rows.push(format!("· route ladder: {legs} leg(s)")),
                }
                let objective = ladder
                    .get("objective")
                    .and_then(|o| o.as_str())
                    .unwrap_or("");
                if !objective.is_empty() {
                    let fallback_legs = ladder
                        .get("fallback_legs")
                        .and_then(|f| f.as_u64())
                        .unwrap_or(legs.saturating_sub(1) as u64);
                    rows.push(format!(
                        "· route objective: {objective} · fallback legs: {fallback_legs}"
                    ));
                }
                if let Some(annotations) = ladder.get("annotations").and_then(|a| a.as_array()) {
                    for note in annotations.iter().filter_map(|n| n.as_str()) {
                        rows.push(format!("⚠ route: {note}"));
                    }
                }
            }
            if failures == 0 {
                rows.push("✓ all checks passed".to_string());
            } else {
                rows.push(format!("✗ {failures} check(s) failed"));
            }
        }
        Err(e) => rows.push(format!("✗ doctor failed: {e}")),
    }
    if let Ok(health) = data.health().await {
        for warning in &health.warnings {
            rows.push(format!("⚠ {warning}"));
        }
    }
    ModalView {
        title: "doctor".to_string(),
        rows,
        scroll: 0,
        footer: "Esc close".to_string(),
        ..Default::default()
    }
}

/// `/services [start|stop|restart] [gateway|telegram]` — background service
/// status and control via vak-ops (docs/design/28-operations.md). Stays
/// local: it drives THIS machine's launchd/systemd units.
pub fn run_services(arg: Option<(String, String)>) -> ModalView {
    let cfg = vak_ops::OpsConfig::detect();
    let mut rows = Vec::new();

    if let Some((action, svc_name)) = arg {
        let svc = match svc_name.as_str() {
            "gateway" => vak_ops::Service::Gateway,
            _ => vak_ops::Service::Telegram,
        };
        match action.as_str() {
            "start" | "stop" | "restart" => {
                let ok = match action.as_str() {
                    "start" => vak_ops::start(svc, &cfg),
                    "stop" => vak_ops::stop(svc, &cfg),
                    _ => {
                        vak_ops::restart(svc, &cfg);
                        true
                    }
                };
                if ok {
                    rows.push(format!("✓ services: {action} {svc_name} — done"));
                } else {
                    rows.push(format!(
                        "✗ services: {action} {svc_name} failed (is it installed? see /services)"
                    ));
                }
            }
            _ => rows.push("usage: /services [start|stop|restart] [gateway|telegram]".to_string()),
        }
    }

    for (name, svc) in [
        ("gateway", vak_ops::Service::Gateway),
        ("telegram", vak_ops::Service::Telegram),
    ] {
        let st = vak_ops::status(svc, &cfg);
        let healthy = name != "gateway" || vak_ops::health_ok(&cfg);
        let extra = if name == "gateway" && st == vak_ops::State::Running && !healthy {
            " · not answering"
        } else {
            ""
        };
        rows.push(format!("{name:<8} {st}{extra}"));
    }
    rows.push("/services start|stop|restart gateway|telegram".to_string());

    ModalView {
        title: "services".to_string(),
        rows,
        scroll: 0,
        footer: "Esc close".to_string(),
        ..Default::default()
    }
}

/// Fetches the shared-renderer Markdown transcript and writes it next to the
/// workspace; returns the path written. Byte-parity with the server-side
/// export is guaranteed by `vak_core::transcript_md`'s shared-renderer test.
pub async fn export_transcript(data: &ClientData, session_id: &str) -> Result<String, String> {
    let content = data.transcript_md(session_id).await?;
    let dir = {
        let cwd = data.cwd_async().await;
        if cwd.as_os_str().is_empty() {
            std::env::current_dir().map_err(|e| e.to_string())?
        } else {
            cwd
        }
    };
    let stem = session_id
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("session")
        .trim_end_matches(".jsonl");
    let path = dir.join(format!("vakcoder-transcript-{stem}.md"));
    std::fs::write(&path, content).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

pub fn report_line(label: &str, value: &str, ok: bool) -> String {
    let mark = if ok { "✓" } else { "✗" };
    format!("  {mark} {label}: {value}")
}

/// `!cmd` plain local shell passthrough — user-invoked, no brokered tool and
/// no sandbox claim; the TUI process runs beside the base and cannot reach
/// the brokered worker (invariant 14).
pub fn run_local_shell(cmd: &str, screen: &mut Screen, theme: &Theme) {
    if cmd.trim().is_empty() {
        screen.dim("usage: !<shell command>");
        return;
    }
    let output = std::process::Command::new("sh").arg("-c").arg(cmd).output();
    let (content, failed) = match output {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&o.stderr);
            if !stderr.trim().is_empty() {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&stderr);
            }
            (truncate_chars(&text, SHELL_OUTPUT_CAP), !o.status.success())
        }
        Err(e) => (format!("sh: {e}"), true),
    };
    screen.clear_input();
    let tail: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    let shown = tail.len().saturating_sub(12);
    let mark = if failed { "✗" } else { "✓" };
    let color = if failed { theme.error } else { theme.success };
    screen.styled(&format!("{mark} !{cmd}"), color);
    for line in &tail[shown..] {
        screen.styled(&format!("  │ {line}"), theme.dim);
    }
}

const SHELL_OUTPUT_CAP: usize = 4000;

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("\n… (output truncated)");
    out
}

fn str_field(v: &serde_json::Value, keys: &[&str], default: &str) -> String {
    for key in keys {
        if let Some(s) = v.get(*key).and_then(|x| x.as_str()) {
            return s.to_string();
        }
    }
    default.to_string()
}
