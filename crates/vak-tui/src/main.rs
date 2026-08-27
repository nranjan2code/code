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
    Some((
        std::env::var("VAKCODER_URL").ok()?,
        std::env::var("VAKCODER_TOKEN").ok()?,
    ))
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
