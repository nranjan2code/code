use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "VakCoder", version, about = "A coding agent harness")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Managed release lifecycle (docs/design/32): install/status/sync
    Self_ {
        #[command(subcommand)]
        action: SelfAction,
    },
    /// Run one prompt headless and print the result
    Exec {
        prompt: String,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long, default_value_t = 40)]
        max_turns: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        permission_mode: Option<String>,
        /// Run in an isolated git worktree off HEAD
        #[arg(long)]
        worktree: bool,
        /// Resume an existing session instead of starting a new one
        #[arg(long)]
        session: Option<String>,
        /// Durable objective for goal mode (docs/design/27 Phase H):
        /// completion is audited against --criteria, never self-reported.
        #[arg(long)]
        goal: Option<String>,
        /// Acceptance criteria, comma-separated. Prefix `verify:` to run a
        /// criterion as a shell command; others are judged from evidence.
        #[arg(long, value_delimiter = ',')]
        criteria: Vec<String>,
        /// Trust this workspace's project config and .env
        #[arg(long)]
        trust: bool,
    },
    /// Show the effective composed configuration
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
    /// List recorded sessions for this project
    Sessions,
    /// Static flow DAGs: list, check, run
    Flow {
        #[command(subcommand)]
        action: FlowAction,
    },
    /// Plan and execute an open-ended task with a dynamic planner
    Plan {
        task: String,
        #[arg(long)]
        yes: bool,
        /// Run in an isolated git worktree off HEAD
        #[arg(long)]
        worktree: bool,
        /// Trust this workspace's project config and .env
        #[arg(long)]
        trust: bool,
    },
    /// Run the built-in eval suite
    Eval {
        /// Write JSON report to this path
        #[arg(long)]
        report: Option<PathBuf>,
        /// Run the live suite against the configured provider (needs API key)
        #[arg(long)]
        live: bool,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
    },
    /// Serve the agent over HTTP+SSE
    Serve {
        #[arg(long, default_value_t = 8901)]
        port: u16,
        /// Enable gateway surface routing regardless of config
        /// (docs/design/22-gateway.md)
        #[arg(long)]
        gateway: bool,
        /// Trust this workspace's project config and .env
        #[arg(long)]
        trust: bool,
    },
    /// Workspace checkpoints: list or restore
    Checkpoints {
        #[command(subcommand)]
        action: CheckpointAction,
    },
    /// Durable memory notes for this workspace: list / add / forget / amend
    Memory {
        #[command(subcommand)]
        action: Option<MemoryAction>,
    },
    /// Review proposed skills: list / promote / reject
    SkillsReview {
        #[command(subcommand)]
        action: SkillsReviewAction,
    },
    /// Bridge a Telegram bot to a running gateway (docs/design/22-gateway.md)
    Telegram {
        /// Gateway base URL, e.g. http://127.0.0.1:8901
        #[arg(long)]
        server: String,
        /// Gateway bearer token (overrides VAKCODER_GATEWAY_TOKEN; the
        /// env var is the normal path so secrets never appear in `ps`)
        #[arg(long)]
        token: Option<String>,
    },
    /// Diagnose provider auth, config warnings, and extensions
    Doctor {
        /// Trust this workspace's project config and .env
        #[arg(long)]
        trust: bool,
    },
    /// Backup your vakcoder home: export or import a directory copy
    Backup {
        #[command(subcommand)]
        action: BackupAction,
    },
    /// Usage digest over cost ledger, memory notes, and skill proposals
    Digest {
        /// Days of history to fold in (clamped to 1..=90)
        #[arg(long, default_value_t = 7)]
        days: u32,
    },
    /// Scheduled tasks stored in ~/.vakcoder/tasks.json: CRUD without the server
    Tasks {
        #[command(subcommand)]
        action: TasksAction,
    },
    /// Durable attention inbox (gateway pushes): list / show / ack / count
    Inbox {
        #[command(subcommand)]
        action: Option<InboxAction>,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum SelfAction {
    /// Copy release binaries into the managed prefix + manifest
    /// Place this build into the managed prefix
    Install {
        /// Managed prefix (default: platform application location)
        #[arg(long)]
        prefix: Option<PathBuf>,
        /// Reinstall even when the prefix already holds this exact build
        #[arg(long)]
        force: bool,
    },
    /// Clear the prefix and place this build fresh
    Reinstall {
        #[arg(long)]
        prefix: Option<PathBuf>,
        #[arg(long)]
        yes: bool,
    },
    /// Check every installed component against the manifest digests
    Verify {
        #[arg(long)]
        prefix: Option<PathBuf>,
    },
    /// Regenerate + reload service units onto the installed binary
    ServicesSync {
        #[arg(long)]
        prefix: Option<PathBuf>,
        /// Service names (default: all); unknown names are reported
        names: Vec<String>,
    },
    /// Drift matrix: build vs manifest vs per-service units
    Status {
        #[arg(long)]
        prefix: Option<PathBuf>,
    },
    /// Reverse of install; --purge also deletes the data home (confirmed)
    Uninstall {
        #[arg(long)]
        prefix: Option<PathBuf>,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        purge: bool,
    },
    /// Opt-in pull-and-replace from a release feed URL
    Update {
        #[arg(long)]
        prefix: Option<PathBuf>,
        /// Release feed URL (default: the [update] url in config)
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        yes: bool,
        /// Report what would change without installing anything
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum InboxAction {
    /// List entries: unread by default, acked included with --all
    List {
        /// Include already-acked entries
        #[arg(long)]
        all: bool,
        /// Maximum rows to print
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Print one entry's body and session/task refs by unique id prefix
    Show { id_prefix: String },
    /// Mark an entry read (idempotent tombstone append)
    Ack { id_prefix: String },
    /// One-line unread total
    Count,
}

#[derive(Subcommand, Debug)]
pub(crate) enum BackupAction {
    /// Copy sessions/memory/checkpoints/ledgers into a directory
    Export {
        /// Destination directory (must not be the vakcoder home itself)
        dir: PathBuf,
        /// Include ~/.vakcoder/.env secrets (a WARNING.txt travels beside them)
        #[arg(long)]
        include_secrets: bool,
    },
    /// Restore a backup directory into the vakcoder home
    Import {
        /// Source directory containing manifest.json
        dir: PathBuf,
        /// What to do when a file already exists: skip or rename
        #[arg(long, default_value = "skip")]
        conflict: String,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum TasksAction {
    /// List tasks with schedule and next-fire preview
    List,
    /// Add a task, or expand a built-in preset (prompt XOR script;
    /// interval XOR cron)
    Add {
        /// Task name (with --preset: overrides the preset's default name)
        #[arg(long, required_unless_present = "preset")]
        name: Option<String>,
        /// Prompt dispatched to the model on each run
        #[arg(long, conflicts_with = "preset")]
        prompt: Option<String>,
        /// Watchdog shell one-liner (zero tokens while stdout stays empty)
        #[arg(long, conflicts_with = "preset")]
        script: Option<String>,
        /// Seconds between runs (default 3600 when no cron given)
        #[arg(long, conflicts_with = "preset")]
        every: Option<u64>,
        /// 5-field cron expression (`m h dom mon dow`, local time)
        #[arg(long, conflicts_with = "preset")]
        cron: Option<String>,
        /// Working directory for runs (default: this directory)
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// Delivery surface for run summaries, e.g. telegram:12345
        #[arg(long)]
        deliver: Option<String>,
        /// Pin this task to a model id (never escalates)
        #[arg(long)]
        model: Option<String>,
        /// Expand a built-in preset: weekly-digest (Mondays 09:00,
        /// runs `digest --days 7` via this binary)
        #[arg(long)]
        preset: Option<String>,
    },
    /// Remove a task by id
    Remove { id: String },
    /// Enable a task
    Enable { id: String },
    /// Disable a task without deleting it
    Disable { id: String },
}

#[derive(Subcommand, Debug)]
pub(crate) enum MemoryAction {
    /// List memory notes (ids are usable with forget/amend)
    List {
        /// Read the global USER.md profile tier instead of this workspace
        #[arg(long)]
        profile: bool,
    },
    /// Remove exactly one note by id
    Forget {
        id: String,
        /// Target the global USER.md profile tier instead of this workspace
        #[arg(long)]
        profile: bool,
    },
    /// Replace one note's body, keeping its provenance header
    Amend {
        id: String,
        text: String,
        /// Target the global USER.md profile tier instead of this workspace
        #[arg(long)]
        profile: bool,
    },
    /// Append a note
    Add {
        text: String,
        /// Note kind, e.g. decision / preference / fact
        #[arg(long, default_value = "note")]
        kind: String,
        #[arg(long, default_value = "")]
        tag: String,
        /// Append to the global USER.md profile tier instead of this workspace
        #[arg(long)]
        profile: bool,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum FlowAction {
    /// List discovered flows
    List,
    /// Convert proven work into a flow file (doc 27 Phase E):
    /// --from accepts a flow-run/plan ledger JSON path or a session id.
    Adopt {
        /// Ledger JSON path, or a session id whose green bash commands
        /// become a chained bash flow.
        from: String,
        /// Name for the adopted flow (written to .vakcoder/flows/)
        #[arg(long)]
        name: String,
        /// Overwrite an existing flow file of the same name
        #[arg(long)]
        force: bool,
    },
    /// Deterministic run-vs-run diff over two ledger JSONs (no model)
    Diff {
        /// Path to first run/plan ledger JSON
        a: std::path::PathBuf,
        /// Path to second run/plan ledger JSON
        b: std::path::PathBuf,
    },
    /// Validate a flow without running it
    Check { name: String },
    /// Run a flow (optionally resuming a previous run)
    Run {
        name: String,
        #[arg(long)]
        resume: bool,
        /// Acknowledge live-file drift and resume the frozen snapshot
        #[arg(long)]
        accept_drift: bool,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// Trust this workspace's project config and .env
        #[arg(long)]
        trust: bool,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ConfigAction {
    Dump,
}

#[derive(Subcommand, Debug)]
pub(crate) enum SkillsReviewAction {
    /// List pending proposals
    List,
    /// Promote a proposal into user-level skills
    Promote { id: String },
    /// Discard a proposal
    Reject { id: String },
}

#[derive(Subcommand, Debug)]
pub(crate) enum CheckpointAction {
    /// List checkpoints for the latest session in this project
    List {
        #[arg(long)]
        session: Option<String>,
    },
    /// Restore a checkpoint into the workspace
    Restore { session: String, seq: u32 },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Command {
        Cli::try_parse_from(std::iter::once("vakcoder").chain(args.iter().copied()))
            .unwrap_or_else(|e| panic!("parse {args:?} failed: {e}"))
            .command
            .expect("subcommand present")
    }

    #[test]
    fn cli_definition_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn doctor_parses_with_and_without_trust() {
        assert!(!matches!(
            parse(&["doctor"]),
            Command::Doctor { trust: true }
        ));
        assert!(matches!(
            parse(&["doctor", "--trust"]),
            Command::Doctor { trust: true }
        ));
    }

    #[test]
    fn backup_export_flags_parse() {
        match parse(&["backup", "export", "/tmp/bk"]) {
            Command::Backup {
                action:
                    BackupAction::Export {
                        dir,
                        include_secrets,
                    },
            } => {
                assert_eq!(dir, PathBuf::from("/tmp/bk"));
                assert!(!include_secrets);
            }
            other => panic!("unexpected: {other:?}"),
        }
        match parse(&["backup", "export", "/tmp/bk", "--include-secrets"]) {
            Command::Backup {
                action:
                    BackupAction::Export {
                        include_secrets, ..
                    },
            } => assert!(include_secrets),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn backup_import_conflict_defaults_to_skip() {
        match parse(&["backup", "import", "/tmp/bk"]) {
            Command::Backup {
                action: BackupAction::Import { conflict, .. },
            } => assert_eq!(conflict, "skip"),
            other => panic!("unexpected: {other:?}"),
        }
        match parse(&["backup", "import", "/tmp/bk", "--conflict", "rename"]) {
            Command::Backup {
                action: BackupAction::Import { conflict, .. },
            } => assert_eq!(conflict, "rename"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn digest_days_default_is_seven() {
        match parse(&["digest"]) {
            Command::Digest { days } => assert_eq!(days, 7),
            other => panic!("unexpected: {other:?}"),
        }
        match parse(&["digest", "--days", "31"]) {
            Command::Digest { days } => assert_eq!(days, 31),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn tasks_add_prompt_and_script_are_separate_flags() {
        match parse(&[
            "tasks",
            "add",
            "--name",
            "nightly",
            "--prompt",
            "tidy",
            "--cron",
            "0 3 * * *",
            "--model",
            "haiku",
        ]) {
            Command::Tasks {
                action:
                    TasksAction::Add {
                        name,
                        prompt,
                        script,
                        every,
                        cron,
                        deliver,
                        model,
                        ..
                    },
            } => {
                assert_eq!(name.as_deref(), Some("nightly"));
                assert_eq!(prompt.as_deref(), Some("tidy"));
                assert_eq!(script, None);
                assert_eq!(every, None);
                assert_eq!(cron.as_deref(), Some("0 3 * * *"));
                assert_eq!(deliver, None);
                assert_eq!(model.as_deref(), Some("haiku"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn tasks_add_preset_parses_without_name_or_content_flags() {
        match parse(&["tasks", "add", "--preset", "weekly-digest"]) {
            Command::Tasks {
                action:
                    TasksAction::Add {
                        name,
                        prompt,
                        script,
                        every,
                        cron,
                        deliver,
                        model,
                        preset,
                        ..
                    },
            } => {
                assert_eq!(preset.as_deref(), Some("weekly-digest"));
                assert_eq!(name, None);
                assert_eq!(prompt, None);
                assert_eq!(script, None);
                assert_eq!(every, None);
                assert_eq!(cron, None);
                assert_eq!(deliver, None);
                assert_eq!(model, None);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn tasks_add_preset_conflicts_are_clap_errors() {
        for extra in [
            vec!["--prompt", "tidy"],
            vec!["--script", "echo tick"],
            vec!["--cron", "0 9 * * 1"],
            vec!["--every", "60"],
        ] {
            let args: Vec<&str> = ["tasks", "add", "--preset", "weekly-digest"]
                .into_iter()
                .chain(extra.iter().copied())
                .collect();
            let err = Cli::try_parse_from(std::iter::once("vakcoder").chain(args))
                .expect_err("preset must reject conflicting flag");
            assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
        }
    }

    #[test]
    fn tasks_add_requires_name_without_preset() {
        let err = Cli::try_parse_from(["vakcoder", "tasks", "add"])
            .expect_err("--name is required without --preset");
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn tasks_enable_disable_remove_take_ids() {
        assert!(matches!(
            parse(&["tasks", "enable", "abc"]),
            Command::Tasks {
                action: TasksAction::Enable { .. }
            }
        ));
        assert!(matches!(
            parse(&["tasks", "disable", "abc"]),
            Command::Tasks {
                action: TasksAction::Disable { .. }
            }
        ));
        match parse(&["tasks", "remove", "abc"]) {
            Command::Tasks {
                action: TasksAction::Remove { id },
            } => assert_eq!(id, "abc"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn inbox_subactions_parse_with_defaults() {
        assert!(matches!(parse(&["inbox"]), Command::Inbox { action: None }));
        match parse(&["inbox", "list", "--all", "--limit", "5"]) {
            Command::Inbox {
                action: Some(InboxAction::List { all, limit }),
            } => {
                assert!(all);
                assert_eq!(limit, 5);
            }
            other => panic!("unexpected: {other:?}"),
        }
        match parse(&["inbox", "show", "abcd1234"]) {
            Command::Inbox {
                action: Some(InboxAction::Show { id_prefix }),
            } => assert_eq!(id_prefix, "abcd1234"),
            other => panic!("unexpected: {other:?}"),
        }
        assert!(matches!(
            parse(&["inbox", "ack", "abcd1234"]),
            Command::Inbox {
                action: Some(InboxAction::Ack { .. })
            }
        ));
        assert!(matches!(
            parse(&["inbox", "count"]),
            Command::Inbox {
                action: Some(InboxAction::Count)
            }
        ));
    }

    #[test]
    fn memory_subactions_carry_profile_flag() {
        match parse(&["memory", "list", "--profile"]) {
            Command::Memory {
                action: Some(MemoryAction::List { profile: true, .. }),
            } => {}
            other => panic!("unexpected: {other:?}"),
        }
        // Bare `memory` keeps its historical listing behavior.
        assert!(matches!(
            parse(&["memory"]),
            Command::Memory { action: None }
        ));
        match parse(&["memory", "amend", "deadbeef", "new body", "--profile"]) {
            Command::Memory {
                action: Some(MemoryAction::Amend { id, text, profile }),
            } => {
                assert_eq!(id, "deadbeef");
                assert_eq!(text, "new body");
                assert!(profile);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
