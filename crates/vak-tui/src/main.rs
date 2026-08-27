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

async fn connection(args: &Args) -> Result<vak_client::Client, String> {
    match (&args.url, &args.token) {
        (Some(url), Some(token)) => {
            return vak_client::Client::new(url.clone(), token.clone())
                .map_err(|error| error.to_string());
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err("--url and --token must be supplied together".to_string());
        }
        (None, None) => {}
    }
    let _ = &args.profile;
    vak_client::GatewayConnection::discover_local_ready()
        .await
        .and_then(|connection| connection.client())
        .map_err(|error| error.to_string())
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let client = match connection(&args).await {
        Ok(connection) => connection,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(2);
        }
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let data =
        match vak_tui::data::ClientData::connect_client(client, cwd.to_string_lossy().into_owned())
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
