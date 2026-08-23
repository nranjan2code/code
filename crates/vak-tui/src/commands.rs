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
    /// `/view <path>` — read a workspace file in a modal. Text is shown with
    /// line numbers; images and binaries report their kind and size rather
    /// than dumping bytes into the terminal.
    View(Option<String>),
    Doctor,
    /// `/services [start|stop|restart] [gateway|telegram]` — background
    /// service control over vak-ops.
    Services(Option<(String, String)>),
    Details,
    Keys(Option<String>),
    Keymap,
    Model(Option<String>),
    Provider(Option<String>),
    /// `/key [provider [SECRET|--remove]]` — inspect, store, or revoke a
    /// provider credential.
    /// Stored via the shared Core into ~/.vakcoder/.env (0600); effective
    /// immediately, no restart.
    Key(Option<String>),
    Config,
    Features,
    Clear,
    /// `/composer [emacs|vim]` — switch or inspect composer keymap mode.
    Composer(Option<String>),
    /// `/subagents` — live subagent attach picker.
    Subagents,
    /// `/a11y [plain|motion|reader [on|off]]` — accessibility toggles.
    A11y(Option<String>),
    /// `/copy` — explicit OSC52 copy of the last response (gated by config).
    Copy,
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
    (
        "key",
        "provider keys: /key provider SECRET · /key provider --remove",
    ),
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
    ("theme", "choose a theme · previews live"),
    ("transcript", "[n] dump recent messages of this session"),
    ("view", "<path> read a workspace file · /cat is an alias"),
    ("doctor", "health check: auth, sandbox, config, extensions"),
    (
        "services",
        "[action svc] gateway & bridge control · bare = status",
    ),
    ("details", "toggle expanded tool result previews"),
    ("keys", "shortcut map · /keys raw captures literal keys"),
    ("keymap", "view bindings · r rebinds interactively"),
    ("composer", "[emacs|vim] modal editing mode"),
    ("subagents", "attach to a running subagent"),
    ("a11y", "accessibility: plain, motion, reader toggles"),
    ("copy", "copy last response via OSC52 if enabled"),
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
        "view" | "cat" => Some(Command::View(arg_opt)),
        "doctor" if arg.is_empty() => Some(Command::Doctor),
        "services" => Some(Command::Services(services_arg(&arg))),
        "details" if arg.is_empty() => Some(Command::Details),
        "keys" => Some(Command::Keys(arg_opt)),
        "keymap" if arg.is_empty() => Some(Command::Keymap),
        "model" => Some(Command::Model(arg_opt)),
        "provider" => Some(Command::Provider(arg_opt)),
        "key" => Some(Command::Key(arg_opt)),
        "config" | "settings" if arg.is_empty() => Some(Command::Config),
        "features" if arg.is_empty() => Some(Command::Features),
        "clear" | "new" if arg.is_empty() => Some(Command::Clear),
        "composer" | "vim" | "emacs" => {
            let arg = match name {
                "vim" => Some("vim".to_string()),
                "emacs" => Some("emacs".to_string()),
                _ => arg_opt,
            };
            Some(Command::Composer(arg))
        }
        "subagents" if arg.is_empty() => Some(Command::Subagents),
        "a11y" | "accessibility" => Some(Command::A11y(arg_opt)),
        "copy" if arg.is_empty() => Some(Command::Copy),
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
  Alt-S          attach to a running subagent\n\
  Alt-Y          copy the last response (OSC52, if enabled)\n\
  Ctrl-U         clear composer\n\
  Ctrl-O         expand a stashed large paste\n\
  Ctrl-G         edit the draft in $EDITOR\n\
  Ctrl-L         clear terminal viewport\n\
  Alt-A          review a pending approval\n\
  Ctrl-C         interrupt run / clear composer\n\
  Esc            remove newest follow-up, then stop\n\
  Ctrl-D         exit\n\
  /vim · /emacs  modal composer editing (hjkl x dd yy p i A …)\n\
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
    out.push_str("\n  /vim switches to modal editing; Alt-S attaches to subagents");
    out
}

pub fn help_rows() -> Vec<String> {
    COMMANDS
        .iter()
        .map(|(name, description)| format!("/{name:<12} {description}"))
        .collect()
}

/// Parses `/a11y [feature [on|off]]`. Returns the feature and the requested
/// state (None = toggle).
pub fn parse_a11y(arg: Option<&str>) -> Result<(A11yFeature, Option<bool>), String> {
    let Some(arg) = arg.map(str::trim).filter(|a| !a.is_empty()) else {
        return Err("usage: /a11y <plain|motion|reader> [on|off]".to_string());
    };
    let mut parts = arg.split_whitespace();
    let feature = match parts.next() {
        Some("plain") => A11yFeature::Plain,
        Some("motion") | Some("reduced-motion") | Some("reduced_motion") => A11yFeature::Motion,
        Some("reader") | Some("screen-reader") | Some("screen_reader") => A11yFeature::Reader,
        Some(other) => {
            return Err(format!(
                "unknown accessibility feature '{other}' — plain, motion, reader"
            ));
        }
        None => unreachable!(),
    };
    let state = match parts.next() {
        None | Some("") => None,
        Some("on" | "true" | "yes") => Some(true),
        Some("off" | "false" | "no") => Some(false),
        Some(other) => {
            return Err(format!("state must be on/off, got '{other}'"));
        }
    };
    Ok((feature, state))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum A11yFeature {
    Plain,
    Motion,
    Reader,
}

impl A11yFeature {
    pub fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Motion => "reduced motion",
            Self::Reader => "screen reader",
        }
    }
}

/// Parse `/services` argument: `action service` in either order, both
/// optional. Returns (action, service) with action defaulting to "status".
pub fn services_arg(arg: &str) -> Option<(String, String)> {
    let arg = arg.trim();
    if arg.is_empty() {
        return None;
    }
    let actions = ["start", "stop", "restart"];
    let services = ["gateway", "telegram", "bridge"];
    let mut action = None;
    let mut svc = None;
    for tok in arg.split_whitespace() {
        let t = tok.to_lowercase();
        if actions.contains(&t.as_str()) {
            action = Some(t);
        } else if services.contains(&t.as_str()) {
            svc = Some(if t == "bridge" { "telegram".into() } else { t });
        }
    }
    Some((
        action.unwrap_or_else(|| "status".into()),
        svc.unwrap_or_else(|| "gateway".into()),
    ))
}
