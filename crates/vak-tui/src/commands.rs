#[derive(Debug)]
pub enum Command {
    Help,
    Exit,
    Cost,
    Context,
    Sessions,
    Model(String),
    Clear,
}

pub fn parse(input: &str) -> Option<Command> {
    let trimmed = input.trim();
    let rest = trimmed.strip_prefix('/')?;
    let mut parts = rest.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim().to_string();
    match name {
        "help" | "?" => Some(Command::Help),
        "exit" | "quit" => Some(Command::Exit),
        "cost" => Some(Command::Cost),
        "context" => Some(Command::Context),
        "sessions" => Some(Command::Sessions),
        "model" if !arg.is_empty() => Some(Command::Model(arg)),
        "clear" | "new" => Some(Command::Clear),
        _ => None,
    }
}

pub fn help_text() -> String {
    "\
/help          show this help
/model <name>  switch model for the next turn
/cost          token totals for this session
/context       show what the model will see next
/sessions      list recorded sessions
/clear         start a fresh session
/exit          quit"
        .to_string()
}
