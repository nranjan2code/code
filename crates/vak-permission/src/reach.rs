//! What a tool reaches and what a rule about it matches, as the tool itself
//! declares them (`vak_tools::Tool::permission`). The engine's mode
//! defaults and the rule language read these, never a tool's name: tools
//! are added and removed at runtime, and a table of names is wrong about
//! every tool it does not list.

/// Where a tool's effect can land. A tool that declares nothing is `Other`:
/// read-only mode denies it and workspace-write asks, as for any unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reach {
    /// Shows the person something Vak already holds; reaches nothing.
    Display,
    /// Coordinates agents inside this run: a question to a parent, a look
    /// at one's own workers. Touches no file, process or network.
    Coordination,
    /// Reads files, confined to the workspace by its path argument.
    ReadPath,
    /// Reads Vak's own records for this Agent (history, skills, its
    /// commitments, a connected account's preview); changes nothing.
    ReadStore,
    /// Writes a file named by its path argument.
    WritePath,
    /// Writes Vak's own learning store for this workspace (memory notes,
    /// skill proposals).
    Learning,
    /// Runs a shell command.
    Command,
    /// Manages the owner's automations: what runs later and when. Listing
    /// reads; a call that sets a shell command (its `script` argument) is
    /// judged as that command, because a script runs unasked when it fires;
    /// anything else writes Vak's own store, as `Learning` does.
    Schedule,
    /// Reaches outside the workspace and this machine.
    Network,
    /// A broker to installed integrations: listing its catalogue reads,
    /// calling one of its tools reaches whatever that tool reaches.
    Integration,
    #[default]
    Other,
}

/// What a rule's pattern is matched against for a tool's call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Subject {
    #[default]
    Nothing,
    /// The path in this argument; `required` when the call is refused
    /// without one rather than read as the workspace.
    Path {
        argument: &'static str,
        required: bool,
    },
    /// The shell command in this argument, matched whole and segment by
    /// segment; an allow rule must cover every segment.
    Command(&'static str),
    /// The text of this argument (a skill's name, a task's label).
    Value(&'static str),
    /// An integration broker's call: `server/tool`.
    IntegrationCall,
}

/// A tool's declaration to the permission engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Declared {
    pub reach: Reach,
    pub subject: Subject,
}

impl Declared {
    /// What this call reaches, where that depends on its arguments: a
    /// `Schedule` call is a read, a command, or a store write.
    pub fn for_call(self, args: &serde_json::Value) -> Self {
        if self.reach != Reach::Schedule {
            return self;
        }
        let arg = |key: &str| args.get(key).and_then(serde_json::Value::as_str);
        if arg("script").is_some_and(|script| !script.trim().is_empty()) {
            Self::new(Reach::Command, Subject::Command("script"))
        } else if arg("action") == Some("list") {
            Self::reach(Reach::ReadStore)
        } else {
            Self::reach(Reach::Learning)
        }
    }

    pub const fn new(reach: Reach, subject: Subject) -> Self {
        Self { reach, subject }
    }

    /// Reach alone, with nothing for a rule's pattern to match.
    pub const fn reach(reach: Reach) -> Self {
        Self {
            reach,
            subject: Subject::Nothing,
        }
    }
}
