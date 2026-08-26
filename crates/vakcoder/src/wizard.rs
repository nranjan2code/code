//! First-run provider wizard (docs/design/29-personal-os.md P3): tty-only,
//! once (marker file), skippable. Saves through `Core::set_provider_key`,
//! the same path the TUI's `/key` command uses, so the key lands in
//! the user `.env` (0600) and takes effect immediately. Every gate fails
//! toward silence — a non-interactive launch must never see this.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use vak_core::Core;

const MARKER_FILE: &str = ".wizard_done";
/// Local providers need no credential and must not block the wizard.
const KEYLESS_PROVIDERS: [&str; 1] = ["ollama"];

pub fn maybe_run_wizard(cwd: &Path) {
    let Ok(core) = Core::new(cwd.to_path_buf()) else {
        return;
    };
    if !wizard_gates_open(&core) {
        return;
    }
    run_wizard(&core);
}

/// All gates in one place so behavior is easy to reason about: interactive
/// terminal, no VAKCODER_NONINTERACTIVE, marker absent, and no keyed
/// provider configured yet.
fn wizard_gates_open(core: &Core) -> bool {
    let stdin_tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let noninteractive = std::env::var_os("VAKCODER_NONINTERACTIVE").is_some();
    if !stdin_tty || noninteractive {
        return false;
    }
    let Some(marker) = vakcoder_home().map(|h| h.join(MARKER_FILE)) else {
        return false;
    };
    if marker.is_file() {
        return false;
    }
    // "No provider key configured yet": no keyed provider is ready.
    !core
        .provider_names()
        .iter()
        .filter(|p| !KEYLESS_PROVIDERS.contains(&p.as_str()))
        .any(|p| core.provider_configured(p))
}

fn vakcoder_home() -> Option<PathBuf> {
    // VAKCODER_HOME override nests everything under one directory (doc 32).
    if let Some(vh) = std::env::var_os("VAKCODER_HOME") {
        return Some(PathBuf::from(vh));
    }
    Some(vak_config::paths::data_home())
}

fn run_wizard(core: &Core) {
    eprintln!();
    eprintln!("welcome to VakCoder — no provider API key is configured yet.");
    let names = core.provider_names();
    for (i, name) in names.iter().enumerate() {
        eprintln!("  {}. {name}", i + 1);
    }

    let choice = loop {
        match prompt_line(&format!(
            "pick a provider to configure [1-{}] (Enter to skip): ",
            names.len()
        )) {
            Ok(answer) => {
                let answer = answer.trim();
                if answer.is_empty() {
                    return finish_wizard(true);
                }
                match answer.parse::<usize>().map(|n| n.checked_sub(1)) {
                    Ok(Some(idx)) if idx < names.len() => break names[idx].clone(),
                    _ => continue,
                }
            }
            Err(_) => return,
        }
    };

    let key = match read_hidden(&format!("API key for {choice} (Enter to skip): ")) {
        Ok(key) if !key.trim().is_empty() => key.trim().to_string(),
        _ => return finish_wizard(true),
    };
    match core.set_provider_key(&choice, &key) {
        Ok(env_var) => {
            eprintln!("✓ {choice} key stored as {env_var} (user .env, owner-only)");
            eprintln!("  switch with /provider {choice} inside the TUI");
            finish_wizard(false);
        }
        Err(e) => {
            // Leave the marker unwritten so the next launch offers a retry.
            eprintln!("error: could not store key: {e}");
        }
    }
}

fn finish_wizard(skipped: bool) {
    if skipped {
        eprintln!("skipped — set a key later via the TUI (/key <provider> SECRET)");
    }
    if let Some(home) = vakcoder_home()
        && std::fs::create_dir_all(&home).is_ok()
        && let Err(e) = std::fs::write(home.join(MARKER_FILE), "")
    {
        eprintln!("note: could not write wizard marker: {e}");
    }
}

fn prompt_line(prompt: &str) -> std::io::Result<String> {
    eprint!("{prompt}");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line)
}

/// Best-effort hidden input: disable terminal echo around the read when
/// stty can do it; otherwise fall back to a plain read rather than failing.
fn read_hidden(prompt: &str) -> std::io::Result<String> {
    eprint!("{prompt}");
    std::io::stderr().flush()?;
    #[cfg(unix)]
    let echo_off = std::process::Command::new("stty")
        .arg("-echo")
        .stdin(std::process::Stdio::inherit())
        .status()
        .is_ok_and(|s| s.success());
    #[cfg(not(unix))]
    let echo_off = false;
    let mut line = String::new();
    let read = std::io::stdin().read_line(&mut line);
    if echo_off {
        let _ = std::process::Command::new("stty")
            .arg("echo")
            .stdin(std::process::Stdio::inherit())
            .status();
        eprintln!();
    }
    read?;
    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_file_name_is_stable() {
        assert_eq!(MARKER_FILE, ".wizard_done");
    }

    #[test]
    fn keyless_provider_list_covers_local_only() {
        // ollama resolves without credentials everywhere.
        assert!(KEYLESS_PROVIDERS.contains(&"ollama"));
        assert!(!KEYLESS_PROVIDERS.contains(&"anthropic"));
    }
}
