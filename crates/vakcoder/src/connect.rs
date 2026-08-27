//! `vakcoder connect` — discover and save connection info for a base.
//!
//! Resolution order:
//! 1. Explicit `--url` + `--token` flags
//! 2. `--profile` (selects from `[connect.profiles]` in user config)
//! 3. Local managed Runtime receipt plus authenticated handshake
//! 4. `[connect]` section in the Runtime config (`<data_home>/config.toml`)
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
        let settings = load_connect_settings();
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

    // 3. The local managed Runtime is the default authority. It takes
    // precedence over a saved remote fallback, which otherwise creates a
    // second, stale connection path for ordinary local use.
    if let Some(resolved) = try_local_runtime() {
        return Ok(resolved);
    }

    // 4. [connect] section in user config is an explicit remote fallback.
    let settings = load_connect_settings();
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

    // 5. Interactive prompt or error
    if atty_is_tty() {
        prompt_onboarding()
    } else {
        Err("no connection info found; pass --url and --token, \
             configure [connect] in <data_home>/config.toml, \
             or start a gateway with `vakcoder serve --gateway`"
            .into())
    }
}

/// Try to discover from a local `runtime/gateway.json`.
fn try_local_runtime() -> Option<Resolved> {
    let connection = vak_client::GatewayConnection::discover_local().ok()?;

    Some(Resolved {
        url: connection.base_url().to_owned(),
        token: connection.token().to_owned(),
        source: "local runtime/gateway.json".into(),
    })
}

/// Interactive onboarding prompt (tty only).
fn prompt_onboarding() -> Result<Resolved, String> {
    eprintln!("No connection to a vakcoder base found.");
    eprintln!("Enter the base URL and auth token to connect.");
    eprintln!("(Find the token in the base's startup output or in <data_home>/config.toml)\n");

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
    let data_home = vak_config::paths::data_home();
    let service = vak_config::ConfigService::new(
        &data_home,
        std::env::current_dir().map_err(|e| e.to_string())?,
    );
    service
        .update(|config| {
            config.connect.url = Some(resolved.url.clone());
            config.connect.token = Some(resolved.token.clone());
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[derive(Default)]
struct ConnectSettings {
    url: Option<String>,
    token: Option<String>,
    profiles: std::collections::BTreeMap<String, Resolved>,
}

fn load_connect_settings() -> ConnectSettings {
    let home = vak_config::paths::data_home();
    let root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let Ok(snapshot) = vak_config::ConfigService::new(home, root).load() else {
        return ConnectSettings::default();
    };
    ConnectSettings {
        url: snapshot.config.connect.url,
        token: snapshot.config.connect.token,
        profiles: snapshot
            .config
            .connect
            .profiles
            .into_iter()
            .map(|(name, profile)| {
                (
                    name,
                    Resolved {
                        url: profile.url,
                        token: profile.token,
                        source: "connect.profile".to_owned(),
                    },
                )
            })
            .collect(),
    }
}

/// Check if stderr is a tty (for interactive prompts).
fn atty_is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stderr().is_terminal()
}
