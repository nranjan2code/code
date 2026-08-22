#[derive(Debug)]
pub enum Command {
    Help,
    Exit,
    Cost,
    Context,
    Sessions,
    Resume(Option<String>),
    Rewind(Option<String>),
    Theme(Option<String>),
    Transcript(Option<String>),
    Doctor,
    Model(String),
    Clear,
}

/// (name, description) — drives help text, completion, and parsing.
/// Names carry no leading slash; the parser/completer add it.
pub const COMMANDS: &[(&str, &str)] = &[
    ("help", "show this help"),
    ("model", "<name> switch model for the next turn"),
    ("cost", "token totals for this session"),
    ("context", "context-window usage"),
    ("sessions", "list recorded sessions"),
    ("resume", "[n|id] continue a past session"),
    ("rewind", "[seq] restore a workspace checkpoint"),
    ("theme", "[dark|light|plain] switch theme now"),
    ("transcript", "[n] dump recent messages of this session"),
    ("doctor", "health check: auth, sandbox, config, extensions"),
    ("clear", "start a fresh session"),
    ("exit", "quit"),
];

pub fn parse(input: &str) -> Option<Command> {
    let trimmed = input.trim();
    let rest = trimmed.strip_prefix('/')?;
    let mut parts = rest.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim().to_string();
    let arg_opt = (!arg.is_empty()).then_some(arg.clone());
    match name {
        "help" | "?" => Some(Command::Help),
        "exit" | "quit" => Some(Command::Exit),
        "cost" => Some(Command::Cost),
        "context" => Some(Command::Context),
        "sessions" => Some(Command::Sessions),
        "resume" => Some(Command::Resume(arg_opt)),
        "rewind" => Some(Command::Rewind(arg_opt)),
        "theme" => Some(Command::Theme(arg_opt)),
        "transcript" => Some(Command::Transcript(arg_opt)),
        "doctor" if arg.is_empty() => Some(Command::Doctor),
        "model" if !arg.is_empty() => Some(Command::Model(arg)),
        "clear" | "new" if arg.is_empty() => Some(Command::Clear),
        _ => None,
    }
}

pub fn help_text() -> String {
    let width = COMMANDS
        .iter()
        .map(|(n, d)| n.len() + usize::from(!d.is_empty()))
        .max()
        .unwrap_or(0);
    let mut out = String::from("\n");
    for (name, desc) in COMMANDS {
        if desc.is_empty() {
            out.push_str(&format!("  /{name}\n"));
        } else {
            out.push_str(&format!("  /{name:<width$}  {desc}\n"));
        }
    }
    out.push_str("  Tab completes commands and @file paths");
    out
}
