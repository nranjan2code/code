//! `vakcoder connect` — discover and save connection info for a base.
//!
//! Resolution order:
//! 1. Explicit `--url` + `--token` flags
//! 2. `--profile` (selects from `[connect.profiles]` in user config)
//! 3. `[connect]` section in user config (`~/.config/vakcoder/config.toml`)
//! 4. Local `runtime/gateway.json` (pid liveness check)
//! 5. Interactive prompt (tty only) or typed error (non-tty)

/// Resolved connection info from the discovery process.
pub(crate) struct Resolved {
    pub url: String,
    pub token: String,
    pub source: String,
}

/// Discover connection info. Returns `Ok(Resolved)` or an error message.
pub(crate) fn discover(
    profile: Option<&str>,
    explicit_url: Option<&str>,
    explicit_token: Option<&str>,
) -> Result<Resolved, String> {
    // 1. Explicit flags
    if let (Some(url), Some(token)) = (explicit_url, explicit_token)
        && !url.is_empty()
        && !token.is_empty()
    {
        return Ok(Resolved {
            url: url.to_string(),
            token: token.to_string(),
            source: "explicit flags".into(),
        });
    }

    // 2. Named profile from config
    if let Some(name) = profile {
        let settings = vak_config::load_connect_settings();
        if let Some(p) = settings.profiles.get(name) {
            return Ok(Resolved {
                url: p.url.clone(),
                token: p.token.clone(),
                source: format!("connect.profile '{name}'"),
            });
        }
        return Err(format!(
            "connect.profile '{name}' not defined in user config"
        ));
    }

    // 3. [connect] section in user config
    let settings = vak_config::load_connect_settings();
    if let (Some(url), Some(token)) = (settings.url, settings.token)
        && !url.is_empty()
        && !token.is_empty()
    {
        return Ok(Resolved {
            url,
            token,
            source: "[connect] in user config".into(),
        });
    }

    // 4. Local runtime/gateway.json
    if let Some(resolved) = try_local_runtime() {
        return Ok(resolved);
    }

    // 5. Interactive prompt or error
    if atty_is_tty() {
        prompt_onboarding()
    } else {
        Err("no connection info found; pass --url and --token, \
             configure [connect] in ~/.config/vakcoder/config.toml, \
             or start a gateway with `vakcoder serve --gateway`"
            .into())
    }
}

/// Try to discover from a local `runtime/gateway.json`.
fn try_local_runtime() -> Option<Resolved> {
    let runtime_path = vak_config::paths::data_home()
        .join("runtime")
        .join("gateway.json");
    let text = std::fs::read_to_string(&runtime_path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&text).ok()?;

    let pid = val.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
    let url = val.get("addr").and_then(|v| v.as_str())?;
    let token = val.get("token").and_then(|v| v.as_str())?;

    if url.is_empty() || token.is_empty() {
        return None;
    }

    // Verify the PID is still alive.
    let alive = std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !alive {
        return None;
    }

    // URL from runtime file is just host:port — prefix http:// if needed.
    let full_url = if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else {
        format!("http://{url}")
    };

    Some(Resolved {
        url: full_url,
        token: token.to_string(),
        source: "local runtime/gateway.json".into(),
    })
}

/// Interactive onboarding prompt (tty only).
fn prompt_onboarding() -> Result<Resolved, String> {
    eprintln!("No connection to a vakcoder base found.");
    eprintln!("Enter the base URL and auth token to connect.");
    eprintln!(
        "(Find the token in the base's startup output or in ~/.config/vakcoder/config.toml)\n"
    );

    eprint!("Base URL (e.g. http://127.0.0.1:8901): ");
    let _ = std::io::Write::flush(&mut std::io::stderr());
    let mut url = String::new();
    std::io::stdin()
        .read_line(&mut url)
        .map_err(|e| e.to_string())?;
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err("URL cannot be empty".into());
    }

    eprint!("Auth token: ");
    let _ = std::io::Write::flush(&mut std::io::stderr());
    let mut token = String::new();
    std::io::stdin()
        .read_line(&mut token)
        .map_err(|e| e.to_string())?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("Token cannot be empty".into());
    }

    Ok(Resolved {
        url,
        token,
        source: "interactive onboarding".into(),
    })
}

/// Save resolved connection info to the user config `[connect]` section.
pub(crate) fn save_to_config(resolved: &Resolved) -> Result<(), String> {
    let config_path = vak_config::global_path().ok_or("cannot determine user config path")?;
    let mut text = if config_path.exists() {
        std::fs::read_to_string(&config_path).map_err(|e| e.to_string())?
    } else {
        String::new()
    };

    // Remove existing [connect] section (simple: find and replace).
    if let Some(start) = text.find("\n[connect]") {
        // Find next section header or end of file.
        let after = &text[start + 1..];
        let next_section = after[1..]
            .find("\n[")
            .map(|i| start + 1 + 1 + i + 1)
            .unwrap_or(text.len());
        text.replace_range(start..next_section, "");
    } else if let Some(start) = text.find("[connect]") {
        let after = &text[start..];
        let next_section = after[1..]
            .find("\n[")
            .map(|i| start + 1 + i + 1)
            .unwrap_or(text.len());
        text.replace_range(start..next_section, "");
    }

    // Ensure trailing newline and append the section.
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!(
        "[connect]\nurl = \"{}\"\ntoken = \"{}\"\n",
        resolved.url, resolved.token
    ));

    // Write atomically.
    if let Some(parent) = config_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&config_path, &text).map_err(|e| e.to_string())?;
    Ok(())
}

/// Check if stderr is a tty (for interactive prompts).
fn atty_is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stderr().is_terminal()
}
