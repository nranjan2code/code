use crate::data::ClientData;
use futures::StreamExt;
use std::path::PathBuf;
use tokio::io::{self, AsyncBufReadExt, BufReader};

pub struct UiConfig {
    pub cwd: PathBuf,
}

pub async fn run(data: ClientData, _config: UiConfig) -> i32 {
    let session = match data.create_session().await {
        Ok(id) => id,
        Err(e) => {
            eprintln!("error creating session: {e}");
            return 1;
        }
    };
    println!(
        "vakcoder Runtime TUI · project={} · session={}",
        data.project.id, session
    );
    println!("Enter a prompt, or /sessions /tasks /memory /config /quit.");
    let mut input = BufReader::new(io::stdin()).lines();
    while let Ok(Some(line)) = input.next_line().await {
        match line.trim() {
            "/quit" | "/exit" => break,
            "/sessions" => match data.sessions().await {
                Ok(rows) => {
                    for row in rows {
                        println!("{} {:?}", row.id, row.status);
                    }
                }
                Err(e) => eprintln!("{e}"),
            },
            "/tasks" => match data.tasks().await {
                Ok(rows) => {
                    for row in rows {
                        println!("{} {}", row.id, row.status);
                    }
                }
                Err(e) => eprintln!("{e}"),
            },
            "/memory" => match data.memory().await {
                Ok(rows) => {
                    for row in rows {
                        println!("{}: {}", row.id, row.text);
                    }
                }
                Err(e) => eprintln!("{e}"),
            },
            "/config" => match data.config().await {
                Ok(config) => println!("revision={} {}", config.revision, config.values),
                Err(e) => eprintln!("{e}"),
            },
            "" => {}
            prompt => match data.run(session.clone(), prompt.to_owned()).await {
                Ok((run, mut events)) => {
                    println!("run {run} started");
                    while let Some(event) = events.next().await {
                        match event {
                            Ok(value) => println!(
                                "{}",
                                serde_json::to_string(&value)
                                    .unwrap_or_else(|_| "event encoding failed".into())
                            ),
                            Err(e) => {
                                eprintln!("{e}");
                                break;
                            }
                        }
                    }
                }
                Err(e) => eprintln!("{e}"),
            },
        }
    }
    0
}
