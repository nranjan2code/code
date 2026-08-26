use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "vakcoder-tui",
    version,
    about = "Terminal client for a vakcoder base"
)]
struct Args {
    #[arg(long)]
    url: Option<String>,
    #[arg(long)]
    token: Option<String>,
    #[arg(long)]
    profile: Option<String>,
}

fn runtime_connection() -> Option<(String, String)> {
    let raw = std::fs::read_to_string(vak_config::paths::data_home().join("runtime/gateway.json"))
        .ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let addr = value.get("addr")?.as_str()?;
    let token = value.get("token")?.as_str()?;
    let url = if addr.starts_with("http://") || addr.starts_with("https://") {
        addr.to_string()
    } else {
        format!("http://{addr}")
    };
    Some((url, token.to_string()))
}

fn connection(args: &Args) -> Result<(String, String), String> {
    match (&args.url, &args.token) {
        (Some(url), Some(token)) => return Ok((url.clone(), token.clone())),
        (Some(_), None) | (None, Some(_)) => {
            return Err("--url and --token must be supplied together".to_string());
        }
        (None, None) => {}
    }
    let settings = vak_config::load_connect_settings();
    if let Some(profile) = &args.profile {
        return settings
            .profiles
            .get(profile)
            .map(|profile| (profile.url.clone(), profile.token.clone()))
            .ok_or_else(|| format!("connect profile {profile:?} is not configured"));
    }
    if let (Some(url), Some(token)) = (settings.url, settings.token) {
        return Ok((url, token));
    }
    runtime_connection().ok_or_else(|| {
        "no vakcoder base found; start the local base or pass --url and --token".to_string()
    })
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let (url, token) = match connection(&args) {
        Ok(connection) => connection,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(2);
        }
    };
    let data = match vak_tui::data::ClientData::connect(&url, &token).await {
        Ok(data) => data,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(2);
        }
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    std::process::exit(vak_tui::run(data, vak_tui::UiConfig { cwd }).await);
}
