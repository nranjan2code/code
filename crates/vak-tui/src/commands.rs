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
    Details,
    Keys,
    Model(Option<String>),
    Provider(Option<String>),
    /// `/key [provider [SECRET]]` — inspect or store a provider credential.
    /// Stored via the shared Core into ~/.vakcoder/.env (0600); effective
    /// immediately, no restart.
    Key(Option<String>),
    Config,
    Features,
    Clear,
}

/// (name, description) — drives help text, completion, and parsing.
/// Names carry no leading slash; the parser/completer add it.
pub const COMMANDS: &[(&str, &str)] = &[
    ("help", "show this help"),
    ("model", "choose a model or provide an exact ID"),
    (
        "provider",
        "choose Anthropic, OpenAI, Google, local, or routed",
    ),
    ("key", "store a provider key: /key provider SECRET"),
    ("config", "effective settings, auth, and config paths"),
    ("settings", "open the settings dashboard"),
    (
        "features",
        "discover sessions, flows, tools, extensions, and reliability",
    ),
    ("cost", "token totals for this session"),
    ("context", "context-window usage"),
    ("sessions", "list recorded sessions"),
    ("resume", "[n|id] continue a past session"),
    ("rewind", "[seq] restore a workspace checkpoint"),
    ("theme", "choose dark, light, neo, rich, Teenage, or plain"),
    ("transcript", "[n] dump recent messages of this session"),
    ("doctor", "health check: auth, sandbox, config, extensions"),
    ("details", "toggle expanded tool result previews"),
    ("keys", "show the complete keyboard shortcut map"),
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
        "details" if arg.is_empty() => Some(Command::Details),
        "keys" if arg.is_empty() => Some(Command::Keys),
        "model" => Some(Command::Model(arg_opt)),
        "provider" => Some(Command::Provider(arg_opt)),
        "key" => Some(Command::Key(arg_opt)),
        "config" | "settings" if arg.is_empty() => Some(Command::Config),
        "features" if arg.is_empty() => Some(Command::Features),
        "clear" | "new" if arg.is_empty() => Some(Command::Clear),
        _ => None,
    }
}

pub fn keys_text() -> &'static str {
    "Keyboard\n\
  Enter          send prompt / steer active run\n\
  Alt-Enter      insert newline\n\
  Tab            complete / queue follow-up while running\n\
  Ctrl-P         open command palette\n\
  Ctrl-R         reverse history search\n\
  Ctrl-T         cycle thinking: indicator / full / off\n\
  Ctrl-Z         undo edit group\n\
  Alt-Z          redo edit group\n\
  Ctrl-W         delete previous word\n\
  Alt-D          delete next word\n\
  Ctrl-A / Home  start of current line\n\
  Ctrl-E / End   end of current line\n\
  Ctrl-K         delete to end of line\n\
  Alt-B / Alt-F  move by word\n\
  Ctrl-U         clear composer\n\
  Ctrl-O         expand a stashed large paste\n\
  Ctrl-G         edit the draft in $EDITOR\n\
  Ctrl-L         clear terminal viewport\n\
  Alt-A          review a pending approval\n\
  Ctrl-C         interrupt run / clear composer\n\
  Esc            remove newest follow-up, then stop\n\
  Ctrl-D         exit\n\
\nPrompt syntax\n\
  @path          attach file contents\n\
  !command       run local shell and share output\n\
  /command       run a terminal command"
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
    out.push_str("\n  @path attaches a file's contents to your message");
    out.push_str("\n  !cmd runs a local shell command and shares its output");
    out.push_str("\n  while running: Enter steers · Tab queues · Esc stops/cancels queue item");
    out.push_str("\n  Ctrl-Z undo · Alt-Z redo · Alt-D delete word · Ctrl-T thinking mode");
    out
}

pub fn help_rows() -> Vec<String> {
    COMMANDS
        .iter()
        .map(|(name, description)| format!("/{name:<12} {description}"))
        .collect()
}
