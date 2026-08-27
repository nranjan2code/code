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
    if let (Ok(url), Ok(token)) = (
        std::env::var("VAKCODER_URL"),
        std::env::var("VAKCODER_TOKEN"),
    ) && !url.is_empty()
        && !token.is_empty()
    {
        return Some((url, token));
    }

    // The managed gateway publishes its authenticated loopback endpoint in
    // this receipt. Keep the standalone TUI on the same discovery path as
    // the CLI and desktop; it still talks only to vak-client after this
    // connection metadata read.
    let raw = std::fs::read_to_string(
        vak_config::paths::data_home()
            .join("runtime")
            .join("gateway.json"),
    )
    .ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let pid = value.get("pid")?.as_u64()?;
    let addr = value.get("addr")?.as_str()?;
    let token = value.get("token")?.as_str()?;
    if addr.is_empty() || token.is_empty() {
        return None;
    }
    let alive = std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !alive {
        return None;
    }
    let url = if addr.starts_with("http://") || addr.starts_with("https://") {
        addr.to_owned()
    } else {
        format!("http://{addr}")
    };
    Some((url, token.to_owned()))
}

fn connection(args: &Args) -> Result<(String, String), String> {
    match (&args.url, &args.token) {
        (Some(url), Some(token)) => return Ok((url.clone(), token.clone())),
        (Some(_), None) | (None, Some(_)) => {
            return Err("--url and --token must be supplied together".to_string());
        }
        (None, None) => {}
    }
    let _ = &args.profile;
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
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let data =
        match vak_tui::data::ClientData::connect(&url, &token, cwd.to_string_lossy().into_owned())
            .await
        {
            Ok(data) => data,
            Err(error) => {
                eprintln!("error: {error}");
                std::process::exit(2);
            }
        };
    std::process::exit(vak_tui::run(data, vak_tui::UiConfig { cwd }).await);
}
