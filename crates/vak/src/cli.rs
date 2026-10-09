use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
// `VAK_VERSION` is composed by build.rs: the release, plus the commit it was
// built from when one was supplied. The commit used to be recorded ONLY in
// install.json — the right place for it, and not the place anyone looks.
// "Which build is this?" is answered by `--version`, and a bare `2.0.1`
// cannot tell two binaries on the same release line apart, which is exactly
// the question that matters when asking whether a fix is in the thing you
// are running.
#[command(name = "vak", version = env!("VAK_VERSION"), about = "Vakyartha — an agent harness")]
pub(crate) struct Cli {
    /// Target a specific workspace directory instead of the current working directory.
    #[arg(long, short = 'C', global = true)]
    pub(crate) workspace: Option<PathBuf>,

    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

/// Which web surface `vak open` targets.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpenSurface {
    /// The workspace client: run tasks, review diffs, answer approvals.
    App,
    /// The operations console: sessions, receipts, services, incidents.
    Admin,
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
        /// Target a specific specialist agent (e.g. 'researcher', 'writer', or custom ID; defaults to built-in 'vak')
        #[arg(long)]
        agent: Option<String>,
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
        /// Permit direct `write` and `edit` calls only for these workspace
        /// paths during this run. Repeat the flag for more than one path.
        #[arg(long = "write-path")]
        write_paths: Vec<PathBuf>,
        /// Run in an isolated git worktree off HEAD
        #[arg(long)]
        worktree: bool,
        /// Resume an existing session instead of starting a new one
        #[arg(long)]
        session: Option<String>,
        /// Track this run as a durable managed work contract
        #[arg(long)]
        managed: bool,
        /// Durable objective for goal mode (docs/design/42-managed-work-contracts.md):
        /// completion is audited against --criteria, never self-reported.
        #[arg(long)]
        goal: Option<String>,
        /// Acceptance criteria, comma-separated. Prefix `verify:` to run a
        /// criterion as a shell command; others are judged from evidence.
        #[arg(long, value_delimiter = ',')]
        criteria: Vec<String>,
        /// Trust this workspace's project config and secret scope
        #[arg(long)]
        trust: bool,
    },
    /// Launch the rich modern terminal console (docs/design/55-rich-terminal-surface.md)
    Term {
        /// Server to connect to (default: the local service's port on 127.0.0.1)
        #[arg(long)]
        server: Option<String>,
        /// Server bearer token (default: the pinned VAK_GATEWAY_TOKEN)
        #[arg(long)]
        token: Option<String>,
        /// Connect to or resume an existing session id
        #[arg(long)]
        session: Option<String>,
    },
    /// Show the effective composed configuration
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
    /// Inspect and edit the layered system prompt
    Prompts {
        #[command(subcommand)]
        action: PromptsAction,
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
        /// Permission mode for this run only, same as `vak exec`. A planner
        /// dispatches model-generated shell through the same engine as any
        /// other turn, so it needs the same boundary control.
        #[arg(long)]
        permission_mode: Option<String>,
        /// Permit direct `write` and `edit` calls only for these workspace
        /// paths during this run. Repeat the flag for more than one path.
        #[arg(long = "write-path")]
        write_paths: Vec<PathBuf>,
        /// Run in an isolated git worktree off HEAD
        #[arg(long)]
        worktree: bool,
        /// Trust this workspace's project config and secret scope
        #[arg(long)]
        trust: bool,
    },
    /// Run the built-in eval suite
    Eval {
        /// Write JSON report to this path
        #[arg(long)]
        report: Option<PathBuf>,
        /// Add this many deterministic, generated scenarios to the built-in suite.
        /// Each run is bounded; use --offset to continue through a larger corpus.
        #[arg(long, default_value_t = 0)]
        generated: usize,
        /// Stable seed used to generate fixture data.
        #[arg(long, default_value_t = 0)]
        seed: u64,
        /// First generated scenario index in this deterministic batch.
        #[arg(long, default_value_t = 0)]
        offset: u64,
        /// Run the live suite against the configured provider (needs API key)
        #[arg(long)]
        live: bool,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
    },
    /// Serve the agent over HTTP+SSE, including the web client at /app
    Serve {
        #[arg(long, default_value_t = 8901)]
        port: u16,
        /// Interface to bind. Defaults to loopback, and anything else
        /// requires `[server] trusted_hosts` to be set — binding a
        /// shell-capable agent to the network is never implicit
        /// (docs/design/48-web-client.md §4.2).
        #[arg(long)]
        host: Option<String>,
        /// Enable gateway surface routing regardless of config
        /// (docs/design/22-gateway.md)
        #[arg(long)]
        gateway: bool,
        /// Trust this workspace's project config and secret scope
        #[arg(long)]
        trust: bool,
    },
    /// Open a running server's web surface in a browser, already signed in
    Open {
        /// Which surface: `app` (the workspace) or `admin` (operations).
        #[arg(value_enum, default_value_t = OpenSurface::App)]
        surface: OpenSurface,
        /// Port the server is listening on. Defaults to the configured one.
        #[arg(long)]
        port: Option<u16>,
        /// Print the URL instead of opening a browser.
        #[arg(long)]
        print: bool,
    },
    /// Explain how a request would be read, and what the runtime would do
    /// about it (docs/design/47-commitment-kernel.md)
    Intent {
        #[command(subcommand)]
        action: IntentAction,
    },
    /// Word, Excel, PowerPoint and PDF files from a script: read, apply ops,
    /// compare and verify, as JSON (docs/design/72-openxml-documents.md,
    /// docs/design/77-pdf-documents.md)
    Office {
        #[command(subcommand)]
        action: OfficeAction,
    },
    /// Durable commitments: what this agent owes, and what closed it
    Commit {
        #[command(subcommand)]
        action: CommitAction,
    },
    /// Delegate authority to a commitment for a bounded time and scope
    Grant {
        /// Commitment id or unique prefix.
        id: String,
        /// Workspace-relative path globs the grant covers (repeatable).
        #[arg(long = "path")]
        paths: Vec<String>,
        /// Tool names the grant covers (repeatable).
        #[arg(long = "tool")]
        tools: Vec<String>,
        /// Lifetime spend the grant permits without asking again.
        #[arg(long)]
        spend_usd: Option<f64>,
        /// Hours until the grant lapses. Omit for no expiry.
        #[arg(long)]
        hours: Option<i64>,
        /// Cap the permission mode while the grant is live. Never raises it.
        #[arg(long, default_value = "workspace-write")]
        permission: String,
        /// What happens to a deferred question nobody answers:
        /// wait | assume | abandon.
        #[arg(long, default_value = "wait")]
        on_silence: String,
        /// Hours before `on_silence` applies.
        #[arg(long, default_value_t = 24)]
        after_hours: u32,
    },
    /// Withdraw a grant. Takes effect immediately.
    Revoke {
        /// Commitment id or unique prefix.
        id: String,
    },
    /// Workspace checkpoints: list or restore
    Checkpoints {
        #[command(subcommand)]
        action: CheckpointAction,
    },
    /// Durable memory notes for this workspace: list / add / forget / amend / consolidate
    Memory {
        #[command(subcommand)]
        action: Option<MemoryAction>,
    },
    /// Manage the semantic entity knowledge graph: list / search / get / delete
    Entities {
        #[command(subcommand)]
        action: Option<EntitiesAction>,
    },
    /// Manage persistent agent specialists and domain templates: list / templates / init
    Agents {
        #[command(subcommand)]
        action: Option<AgentsAction>,
    },
    /// Export session transcript or living outcome canvas to standalone HTML or markdown
    Export {
        /// Session ID to export
        session_id: String,
        /// Export as an interactive self-contained HTML document
        #[arg(long)]
        html: bool,
        /// Optional destination path to write output (defaults to stdout)
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Review proposed skills: list / promote / reject
    SkillsReview {
        #[command(subcommand)]
        action: SkillsReviewAction,
    },
    /// Validate Agent Skills files in a project or explicit path
    Skills {
        #[command(subcommand)]
        action: SkillsAction,
    },
    /// Inspect and manage immutable, disabled-by-default plugin packages
    Plugins {
        #[command(subcommand)]
        action: PluginAction,
    },
    /// Bridge a Telegram bot to a running gateway (docs/design/22-gateway.md)
    Telegram {
        /// Gateway base URL, e.g. http://127.0.0.1:8901
        #[arg(long)]
        server: String,
        /// Gateway bearer token (overrides VAK_GATEWAY_TOKEN; the
        /// env var is the normal path so secrets never appear in `ps`)
        #[arg(long)]
        token: Option<String>,
        /// Run as this bot identity instead of the legacy single
        /// `TELEGRAM_BOT_TOKEN` slot (multi-bot-per-channel, docs/design/34).
        /// Its token is read from the env var recorded for this id in the
        /// admin console's Bots list. Run one `vak telegram --bot-id ...`
        /// process per bot to have more than one Telegram bot live at once.
        #[arg(long)]
        bot_id: Option<String>,
    },
    /// Bridge a Discord bot to a running gateway (docs/design/34 Phase 3)
    Discord {
        /// Gateway base URL, e.g. http://127.0.0.1:8901
        #[arg(long)]
        server: String,
        /// Gateway bearer token (overrides VAK_GATEWAY_TOKEN; the
        /// env var is the normal path so secrets never appear in `ps`)
        #[arg(long)]
        token: Option<String>,
        /// See `telegram --bot-id`.
        #[arg(long)]
        bot_id: Option<String>,
    },
    /// Bridge a Slack bot to a running gateway (docs/design/34 Phase 3)
    Slack {
        /// Gateway base URL, e.g. http://127.0.0.1:8901
        #[arg(long)]
        server: String,
        /// Gateway bearer token (overrides VAK_GATEWAY_TOKEN; the
        /// env var is the normal path so secrets never appear in `ps`)
        #[arg(long)]
        token: Option<String>,
        /// See `telegram --bot-id`.
        #[arg(long)]
        bot_id: Option<String>,
    },
    /// Guided first run: choose a workspace, connect a model, activate services
    Setup {
        #[command(subcommand)]
        action: Option<SetupAction>,
        /// Print the URL and wait instead of opening a browser
        #[arg(long)]
        no_browser: bool,
        /// Print the URL for tunnelling to a headless host, then wait
        #[arg(long)]
        print_url: bool,
        /// Run the whole flow as terminal prompts, with no browser
        #[arg(long)]
        terminal: bool,
        /// Take every choice from the environment and fail on any missing
        /// one, instead of prompting
        #[arg(long)]
        non_interactive: bool,
    },
    /// Diagnose provider auth, config warnings, and extensions
    Doctor {
        /// Trust this workspace's project config and secret scope
        #[arg(long)]
        trust: bool,
        /// Act on failing checks that have a known fix, then re-check
        #[arg(long)]
        repair: bool,
    },
    /// Backup your vak home: export or import a directory copy
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
    /// Automations (triggers): list, add, enable, disable and remove them
    /// without the server
    Triggers {
        #[command(subcommand)]
        action: TriggersAction,
    },
    /// Durable attention inbox (gateway pushes): list / show / ack / count
    Inbox {
        #[command(subcommand)]
        action: Option<InboxAction>,
    },
    /// What Vak keeps and how it is kept: status, usage, the retention
    /// plan and a pass (gc), erasure and its receipts, holds, an
    /// integrity check (verify), and a plain view (cat, grep).
    Data {
        #[command(subcommand)]
        action: Option<DataAction>,
    },
    /// Keep a second copy of your data in a folder, so another of your
    /// machines can take the work over (shows where it stands when given
    /// no verb)
    Sync {
        #[command(subcommand)]
        action: Option<SyncAction>,
    },
    /// Run records: every unit of work, whatever caused it (list / show)
    Runs {
        #[command(subcommand)]
        action: Option<RunsAction>,
    },
    /// Effect records: every action taken outside Vak (list / show /
    /// reconcile)
    Effects {
        #[command(subcommand)]
        action: Option<EffectsAction>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum PluginScopeArg {
    User,
    Workspace,
}

#[derive(Subcommand, Debug)]
pub(crate) enum SkillsAction {
    /// Validate one SKILL.md or every SKILL.md below a directory
    Validate {
        path: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum MarketplaceTrustArg {
    ManualReview,
    PinnedCommit,
    LocalOnly,
}

#[derive(Subcommand, Debug)]
pub(crate) enum PluginAction {
    /// Inspect and hash a Codex, Claude, Copilot, or Cursor catalog snapshot
    CatalogInspect {
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Register a catalog snapshot for explicit review; it starts disabled
    CatalogRegister {
        path: PathBuf,
        #[arg(long, default_value = "local catalog")]
        label: String,
        #[arg(long, value_enum, default_value_t = MarketplaceTrustArg::ManualReview)]
        trust: MarketplaceTrustArg,
        #[arg(long, value_enum, default_value_t = PluginScopeArg::User)]
        scope: PluginScopeArg,
        #[arg(long)]
        json: bool,
    },
    /// List registered catalog snapshots and their trust state
    CatalogSources {
        #[arg(long, value_enum, default_value_t = PluginScopeArg::User)]
        scope: PluginScopeArg,
        #[arg(long)]
        json: bool,
    },
    /// Search entries in registered marketplace catalog snapshots
    CatalogSearch {
        query: String,
        #[arg(long, value_enum, default_value_t = PluginScopeArg::User)]
        scope: PluginScopeArg,
        #[arg(long)]
        json: bool,
    },
    /// Materialize and install one explicitly selected catalog entry.
    CatalogInstall {
        catalog: PathBuf,
        name: String,
        #[arg(long, value_enum, default_value_t = PluginScopeArg::User)]
        scope: PluginScopeArg,
        #[arg(long)]
        allow_unlicensed: bool,
        #[arg(long)]
        json: bool,
    },
    /// Inspect a local package without installing or executing it
    Inspect {
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Copy a reviewed local package into immutable storage, disabled
    Install {
        path: PathBuf,
        #[arg(long, value_enum, default_value_t = PluginScopeArg::User)]
        scope: PluginScopeArg,
        /// Proceed when the package declares no license
        #[arg(long)]
        allow_unlicensed: bool,
        #[arg(long)]
        json: bool,
    },
    /// Inspect and stage a new immutable generation without activating it
    Update {
        path: PathBuf,
        #[arg(long, value_enum, default_value_t = PluginScopeArg::User)]
        scope: PluginScopeArg,
        #[arg(long)]
        allow_unlicensed: bool,
        #[arg(long)]
        json: bool,
    },
    /// Enable the currently selected generation
    Enable {
        name: String,
        #[arg(long, value_enum, default_value_t = PluginScopeArg::User)]
        scope: PluginScopeArg,
        #[arg(long)]
        json: bool,
    },
    /// Disable a plugin without uninstalling it
    Disable {
        name: String,
        #[arg(long, value_enum, default_value_t = PluginScopeArg::User)]
        scope: PluginScopeArg,
        #[arg(long)]
        json: bool,
    },
    /// Select the previous immutable generation and leave it disabled
    Rollback {
        name: String,
        #[arg(long, value_enum, default_value_t = PluginScopeArg::User)]
        scope: PluginScopeArg,
        #[arg(long)]
        json: bool,
    },
    /// List every immutable generation retained for a plugin
    Versions {
        name: String,
        #[arg(long, value_enum, default_value_t = PluginScopeArg::User)]
        scope: PluginScopeArg,
        #[arg(long)]
        json: bool,
    },
    /// List installed packages for one scope
    List {
        #[arg(long, value_enum, default_value_t = PluginScopeArg::User)]
        scope: PluginScopeArg,
        #[arg(long)]
        json: bool,
    },
    /// Show append-only install and removal provenance
    Audit {
        #[arg(long, value_enum, default_value_t = PluginScopeArg::User)]
        scope: PluginScopeArg,
        #[arg(long)]
        json: bool,
    },
    /// Unregister and remove one immutable package generation
    Remove {
        name: String,
        #[arg(long, value_enum, default_value_t = PluginScopeArg::User)]
        scope: PluginScopeArg,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum SelfAction {
    /// Digest every durable file the state registry declares, or check a
    /// snapshot taken before an update against what is on disk now
    State {
        /// Compare the current state against this snapshot and report any
        /// entry the update contract forbids changing
        #[arg(long)]
        verify: Option<std::path::PathBuf>,
    },
    /// Place this build into the managed prefix, with a manifest
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
pub(crate) enum SyncAction {
    /// Where this machine stands against its remote folder
    Status,
    /// Name the folder the copy is kept in (it must exist already)
    Setup { folder: std::path::PathBuf },
    /// Bring the remote copy up to date with this machine
    Now,
    /// Bring this machine up to the remote copy. Vakyartha must be
    /// started again afterwards
    Pull {
        /// Replace work on this machine that was never pushed
        #[arg(long)]
        discard: bool,
    },
    /// Hand the work over to your other machine: a last push, then this
    /// machine stands by. Refused while anything is running
    Handover,
    /// Take the work over on this machine. Vakyartha must be started
    /// again afterwards
    Takeover {
        /// The other machine is lost and cannot hand over. What it never
        /// pushed is not brought here
        #[arg(long)]
        force: bool,
        /// Replace work on this machine that was never pushed
        #[arg(long)]
        discard: bool,
    },
    /// Say where a project made on your other machine is on this one
    Place {
        /// The project's id (spc_…) or its name
        project: String,
        /// Its folder on this machine
        folder: std::path::PathBuf,
    },
    /// Stop using the remote folder. Nothing in it is touched
    Forget,
    /// The key file a second machine needs to read the remote copy
    Key {
        #[command(subcommand)]
        action: KeyFileAction,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum KeyFileAction {
    /// Write this machine's keys to a file, protected by a passphrase
    /// you choose. Carry it to the other machine yourself
    Export { file: std::path::PathBuf },
    /// Take the keys from a key file made on your other machine. Only on
    /// a new install
    Import { file: std::path::PathBuf },
}

#[derive(Subcommand, Debug)]
pub(crate) enum DataAction {
    /// The reconciler's mode, the retention that applies, and totals
    Status,
    /// Measured storage by root, class and owner
    Usage,
    /// Run retention now. It removes what is due only when `[lifecycle]
    /// mode = "commit"`; otherwise, or with --dry-run, it shows the plan
    Gc {
        /// Show the plan and remove nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Erase a conversation in the trash for good (shows what will be
    /// destroyed and asks you to type its title), what one guest wrote
    /// in a conversation, or what a disconnected account returned
    Erase {
        /// The conversation's id; the account's with --scope account; the
        /// Agent's with --scope agent; the project's (spc_…) with --scope
        /// project; the word `everything` with --scope install
        session: String,
        /// What is erased: conversation, guest, account, agent, project or
        /// install (everything Vakyartha stored on this machine)
        #[arg(long, default_value = "conversation")]
        scope: String,
        /// With --scope guest: the guest whose contributions are erased
        #[arg(long)]
        guest: Option<String>,
    },
    /// Put a conversation on hold, so that nothing erases it
    Hold {
        /// The conversation's id
        session: String,
        /// Release the hold
        #[arg(long)]
        release: bool,
    },
    /// The signed receipt of every erasure
    Receipts,
    /// Where the keys that protect your data are kept and which is in
    /// use; --rotate starts a new one and re-protects everything under it
    Keys {
        #[arg(long)]
        rotate: bool,
    },
    /// Check that nothing stored was changed or lost: every conversation
    /// and record chain, the keys, the receipts, and whether search is
    /// behind. Changes nothing
    Verify {
        /// Print the report as JSON
        #[arg(long)]
        json: bool,
    },
    /// Rebuild search and lineage from the records
    RebuildCatalog,
    /// How long each kind of data is kept. With --set, change a keep
    /// time; a shorter one shows what it would remove and asks first
    Rules {
        /// A kind and its keep time in days, as kind=days (for example
        /// trash=14). Repeat for more than one
        #[arg(long = "set", value_name = "KIND=DAYS")]
        set: Vec<String>,
        /// Go back to the default keep times
        #[arg(long)]
        reset: bool,
    },
    /// Print a conversation as plain text
    Cat {
        /// The conversation's id
        session: String,
    },
    /// Search everything Vak keeps, as plain text
    Grep {
        /// What to look for
        text: String,
        /// How many results to print
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// What retention would remove or move to the trash now (a dry run)
    Plan {
        /// Print the plan as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum RunsAction {
    /// List runs, newest first
    List {
        /// Only runs in this state (running, completed, failed, cancelled,
        /// abandoned, skipped)
        #[arg(long)]
        status: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Show one run as JSON
    Show { id: String },
}

#[derive(Subcommand, Debug)]
pub(crate) enum EffectsAction {
    /// List effects, newest first
    List {
        /// Only effects in this state (held, queued, sending, sent,
        /// retrying, failed, unknown, superseded)
        #[arg(long)]
        status: Option<String>,
        /// Only the effects of this run
        #[arg(long)]
        run: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Show one effect as JSON
    Show { id: String },
    /// Say whether an effect nobody could prove was sent
    Reconcile {
        id: String,
        #[arg(long, value_enum)]
        outcome: ReconcileArg,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ReconcileArg {
    Sent,
    NotSent,
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

/// `vak setup` actions. Bare `vak setup` runs the guided flow; `status`
/// is the read-only projection every surface shares
/// (docs/design/46-stabilization-install-and-onboarding.md).
#[derive(Subcommand, Debug)]
pub(crate) enum SetupAction {
    /// Install the Shared starter skills and plugins into ~/vak-home
    Seed,
    /// What is configured, what is not, and the one repair for each gap
    Status {
        /// Emit the readiness projection as JSON instead of a report
        #[arg(long)]
        json: bool,
        /// Inspect a managed install at this prefix
        #[arg(long)]
        prefix: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum BackupAction {
    /// Copy sessions/memory/checkpoints/ledgers into a directory
    Export {
        /// Destination directory (must not be the vak home itself)
        dir: PathBuf,
        /// Include the encrypted credential store, when this host uses one
        /// (a WARNING.txt travels beside it; a host using the OS keychain
        /// has nothing here to include)
        #[arg(long)]
        include_secrets: bool,
    },
    /// Restore a backup directory into the vak home
    Import {
        /// Source directory containing manifest.json
        dir: PathBuf,
        /// What to do when a file already exists: skip or rename
        #[arg(long, default_value = "skip")]
        conflict: String,
    },
}

#[derive(clap::Args, Debug)]
pub(crate) struct TriggerAdd {
    /// Its name (with --preset: overrides the preset's default name)
    #[arg(long, required_unless_present = "preset")]
    pub(crate) name: Option<String>,
    /// Prompt dispatched to the model on each run
    #[arg(long, conflicts_with = "preset")]
    pub(crate) prompt: Option<String>,
    /// Watchdog shell one-liner (zero tokens while stdout stays empty)
    #[arg(long, conflicts_with = "preset")]
    pub(crate) script: Option<String>,
    /// Seconds between runs (default 3600 when no cron given)
    #[arg(long, conflicts_with = "preset")]
    pub(crate) every: Option<u64>,
    /// 5-field cron expression (`m h dom mon dow`)
    #[arg(long, conflicts_with = "preset")]
    pub(crate) cron: Option<String>,
    /// IANA timezone the cron is read in (default: this machine's)
    #[arg(long)]
    pub(crate) timezone: Option<String>,
    /// Working directory for runs (default: this directory)
    #[arg(long)]
    pub(crate) cwd: Option<PathBuf>,
    /// Delivery surface for run summaries, e.g. telegram:12345
    #[arg(long)]
    pub(crate) deliver: Option<String>,
    /// Pin a prompt automation to a model id (never escalates)
    #[arg(long)]
    pub(crate) model: Option<String>,
    /// Expand a built-in preset: weekly-digest (Mondays 09:00,
    /// runs `digest --days 7` via this binary)
    #[arg(long)]
    pub(crate) preset: Option<String>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum TriggersAction {
    /// List automations with their schedule, next run and last run
    List,
    /// Add an automation, or expand a built-in preset (prompt XOR script;
    /// interval XOR cron)
    Add(Box<TriggerAdd>),
    /// Remove an automation by id
    Remove { id: String },
    /// Enable an automation
    Enable { id: String },
    /// Disable an automation without deleting it
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
    /// Run autonomous memory consolidation: promotes recurring procedures to invariants and detects conflicts
    Consolidate,
}

#[derive(Subcommand, Debug)]
pub(crate) enum EntitiesAction {
    /// List entities in this workspace (or global)
    List {
        /// Read the global entities tier instead of this workspace
        #[arg(long)]
        global: bool,
    },
    /// Search entities across names, summaries, attributes, and relations
    Search {
        query: String,
        /// Filter by entity category (e.g. 'service', 'database', 'person')
        #[arg(long)]
        entity_type: Option<String>,
        /// Search the global entities tier instead of this workspace
        #[arg(long)]
        global: bool,
    },
    /// Get details of an entity by id
    Get {
        id: String,
        /// Target the global entities tier instead of this workspace
        #[arg(long)]
        global: bool,
    },
    /// Delete an entity by id
    Delete {
        id: String,
        /// Target the global entities tier instead of this workspace
        #[arg(long)]
        global: bool,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum AgentsAction {
    /// List configured agent specialists
    List {
        /// Target the global agents tier (~/vak-home/.vak/agents.json)
        #[arg(long)]
        global: bool,
    },
    /// List available built-in domain specialist templates (researcher, writer, operator, analyst)
    Templates,
    /// Instantiate a new specialist agent from a domain template
    Init {
        /// Template ID to instantiate: researcher | writer | operator | analyst
        #[arg(long)]
        template: String,
        /// Unique ID for the new agent (e.g. 'data-lead')
        #[arg(long)]
        id: String,
        /// Optional custom display name
        #[arg(long)]
        name: Option<String>,
        /// Target the global agents tier (~/vak-home/.vak/agents.json)
        #[arg(long)]
        global: bool,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum FlowAction {
    /// List discovered flows
    List,
    /// Convert proven work into a flow file (docs/design/10-flows.md):
    /// --from accepts a flow-run/plan ledger JSON path or a session id.
    Adopt {
        /// Ledger JSON path, or a session id whose green bash commands
        /// become a chained bash flow.
        from: String,
        /// Name for the adopted flow (written to .vak/flows/)
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
        /// Trust this workspace's project config and secret scope
        #[arg(long)]
        trust: bool,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ConfigAction {
    /// Print the effective composed configuration
    Dump,
    /// Show the permission mode, approval mode, and rule lists in force
    Permissions,
    /// Set the permission mode, persisted to a config layer
    SetMode {
        /// read-only | workspace-write | full-access
        mode: String,
        #[arg(long, default_value = "project")]
        scope: PromptScope,
    },
    /// Set how `Ask` decisions are resolved, persisted to a config layer
    SetApproval {
        /// ask | approve-safe | auto-approve
        mode: String,
        #[arg(long, default_value = "project")]
        scope: PromptScope,
    },
}

/// Scope for a prompt edit. Matches the wire names the admin API uses;
/// interfaces label these "Shared" and "This project" (docs/design/05-config.md).
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PromptScope {
    /// ~/vak-home — the baseline every project inherits
    User,
    /// this workspace only
    Project,
}

#[derive(Subcommand, Debug)]
pub(crate) enum PromptsAction {
    /// Print the assembled prompt, or one layer's own text
    Show {
        /// Show only this block (identity, operating-rules, guardrails)
        block: Option<String>,
        /// Print the layer's own text instead of the assembled result
        #[arg(long)]
        scope: Option<PromptScope>,
        /// Annotate each contributing layer
        #[arg(long)]
        provenance: bool,
    },
    /// Open a block in $EDITOR and save it to the chosen scope
    Edit {
        block: String,
        #[arg(long, default_value = "project")]
        scope: PromptScope,
    },
    /// Set a block from a file or stdin (`-`), non-interactively
    Set {
        block: String,
        /// File to read; `-` reads stdin
        from: String,
        #[arg(long, default_value = "project")]
        scope: PromptScope,
    },
    /// Delete this layer's block and resume inheritance
    Reset {
        block: String,
        #[arg(long, default_value = "project")]
        scope: PromptScope,
    },
    /// Show what this workspace changed against the shipped default
    Diff,
    /// Render the exact prompt a given surface and role would receive
    Preview {
        /// cli, desktop, server, background, worker, or a chat channel
        #[arg(long, default_value = "cli")]
        surface: String,
        /// Named agent role to apply
        #[arg(long)]
        role: Option<String>,
    },
    /// List named agent roles defined for this workspace
    Roles,
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
        Cli::try_parse_from(std::iter::once("vak").chain(args.iter().copied()))
            .unwrap_or_else(|e| panic!("parse {args:?} failed: {e}"))
            .command
            .expect("subcommand present")
    }

    #[test]
    fn cli_definition_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn eval_accepts_reproducible_generated_batches() {
        assert!(matches!(
            parse(&[
                "eval",
                "--generated",
                "500",
                "--seed",
                "42",
                "--offset",
                "1000"
            ]),
            Command::Eval {
                generated: 500,
                seed: 42,
                offset: 1000,
                live: false,
                ..
            }
        ));
    }

    #[test]
    fn doctor_parses_with_and_without_trust() {
        assert!(!matches!(
            parse(&["doctor"]),
            Command::Doctor { trust: true, .. }
        ));
        assert!(matches!(
            parse(&["doctor", "--trust"]),
            Command::Doctor { trust: true, .. }
        ));
    }

    #[test]
    fn doctor_parses_repair() {
        assert!(matches!(
            parse(&["doctor", "--repair"]),
            Command::Doctor { repair: true, .. }
        ));
        assert!(!matches!(
            parse(&["doctor"]),
            Command::Doctor { repair: true, .. }
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
    fn triggers_add_prompt_and_script_are_separate_flags() {
        match parse(&[
            "triggers",
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
            Command::Triggers {
                action: TriggersAction::Add(add),
            } => {
                let TriggerAdd {
                    name,
                    prompt,
                    script,
                    every,
                    cron,
                    deliver,
                    model,
                    ..
                } = *add;
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
    fn triggers_add_preset_parses_without_name_or_content_flags() {
        match parse(&["triggers", "add", "--preset", "weekly-digest"]) {
            Command::Triggers {
                action: TriggersAction::Add(add),
            } => {
                let TriggerAdd {
                    name,
                    prompt,
                    script,
                    every,
                    cron,
                    deliver,
                    model,
                    preset,
                    ..
                } = *add;
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
    fn triggers_add_preset_conflicts_are_clap_errors() {
        for extra in [
            vec!["--prompt", "tidy"],
            vec!["--script", "echo tick"],
            vec!["--cron", "0 9 * * 1"],
            vec!["--every", "60"],
        ] {
            let args: Vec<&str> = ["triggers", "add", "--preset", "weekly-digest"]
                .into_iter()
                .chain(extra.iter().copied())
                .collect();
            let err = Cli::try_parse_from(std::iter::once("vak").chain(args))
                .expect_err("preset must reject conflicting flag");
            assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
        }
    }

    #[test]
    fn triggers_add_requires_name_without_preset() {
        let err = Cli::try_parse_from(["vak", "triggers", "add"])
            .expect_err("--name is required without --preset");
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn triggers_enable_disable_remove_take_ids() {
        assert!(matches!(
            parse(&["triggers", "enable", "abc"]),
            Command::Triggers {
                action: TriggersAction::Enable { .. }
            }
        ));
        assert!(matches!(
            parse(&["triggers", "disable", "abc"]),
            Command::Triggers {
                action: TriggersAction::Disable { .. }
            }
        ));
        match parse(&["triggers", "remove", "abc"]) {
            Command::Triggers {
                action: TriggersAction::Remove { id },
            } => assert_eq!(id, "abc"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn runs_subactions_parse() {
        assert!(matches!(parse(&["runs"]), Command::Runs { action: None }));
        match parse(&["runs", "list", "--status", "failed", "--limit", "5"]) {
            Command::Runs {
                action: Some(RunsAction::List { status, limit }),
            } => {
                assert_eq!(status.as_deref(), Some("failed"));
                assert_eq!(limit, 5);
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(matches!(
            parse(&["runs", "show", "run_x"]),
            Command::Runs {
                action: Some(RunsAction::Show { .. })
            }
        ));
    }

    #[test]
    fn effects_subactions_parse() {
        assert!(matches!(
            parse(&["effects"]),
            Command::Effects { action: None }
        ));
        match parse(&["effects", "list", "--status", "unknown", "--run", "run_x"]) {
            Command::Effects {
                action: Some(EffectsAction::List { status, run, limit }),
            } => {
                assert_eq!(status.as_deref(), Some("unknown"));
                assert_eq!(run.as_deref(), Some("run_x"));
                assert_eq!(limit, 50);
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(matches!(
            parse(&["effects", "reconcile", "eff_x", "--outcome", "not-sent"]),
            Command::Effects {
                action: Some(EffectsAction::Reconcile {
                    outcome: ReconcileArg::NotSent,
                    ..
                })
            }
        ));
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

    #[test]
    fn memory_consolidate_and_entities_and_export_commands_parse() {
        assert!(matches!(
            parse(&["memory", "consolidate"]),
            Command::Memory {
                action: Some(MemoryAction::Consolidate),
            }
        ));

        assert!(matches!(
            parse(&["entities"]),
            Command::Entities { action: None }
        ));

        match parse(&[
            "entities",
            "search",
            "postgres",
            "--entity-type",
            "database",
        ]) {
            Command::Entities {
                action:
                    Some(EntitiesAction::Search {
                        query,
                        entity_type,
                        global,
                    }),
            } => {
                assert_eq!(query, "postgres");
                assert_eq!(entity_type.as_deref(), Some("database"));
                assert!(!global);
            }
            other => panic!("unexpected: {other:?}"),
        }

        match parse(&["export", "sess-1234", "--html"]) {
            Command::Export {
                session_id,
                html,
                out,
            } => {
                assert_eq!(session_id, "sess-1234");
                assert!(html);
                assert!(out.is_none());
            }
            other => panic!("unexpected: {other:?}"),
        }

        assert!(matches!(
            parse(&["agents"]),
            Command::Agents { action: None }
        ));
        assert!(matches!(
            parse(&["agents", "templates"]),
            Command::Agents {
                action: Some(AgentsAction::Templates)
            }
        ));
        match parse(&[
            "agents",
            "init",
            "--template",
            "researcher",
            "--id",
            "research-lead",
            "--name",
            "Dr. Lead",
        ]) {
            Command::Agents {
                action:
                    Some(AgentsAction::Init {
                        template,
                        id,
                        name,
                        global,
                    }),
            } => {
                assert_eq!(template, "researcher");
                assert_eq!(id, "research-lead");
                assert_eq!(name.as_deref(), Some("Dr. Lead"));
                assert!(!global);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn plugin_lifecycle_commands_parse() {
        assert!(matches!(
            parse(&["plugins", "catalog-inspect", "./catalog"]),
            Command::Plugins {
                action: PluginAction::CatalogInspect { .. }
            }
        ));
        assert!(matches!(
            parse(&["plugins", "inspect", "./sample", "--json"]),
            Command::Plugins {
                action: PluginAction::Inspect { json: true, .. }
            }
        ));
        assert!(matches!(
            parse(&[
                "plugins",
                "install",
                "./sample",
                "--scope",
                "workspace",
                "--allow-unlicensed"
            ]),
            Command::Plugins {
                action: PluginAction::Install {
                    scope: PluginScopeArg::Workspace,
                    allow_unlicensed: true,
                    ..
                }
            }
        ));
        assert!(matches!(
            parse(&["plugins", "audit"]),
            Command::Plugins {
                action: PluginAction::Audit {
                    scope: PluginScopeArg::User,
                    ..
                }
            }
        ));
    }

    #[test]
    fn exec_accepts_repeatable_write_scope_paths() {
        match parse(&[
            "exec",
            "update files",
            "--write-path",
            "src/lib.rs",
            "--write-path",
            "Cargo.toml",
        ]) {
            Command::Exec { write_paths, .. } => {
                assert_eq!(
                    write_paths,
                    vec![PathBuf::from("src/lib.rs"), PathBuf::from("Cargo.toml")]
                );
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}

#[derive(Subcommand, Debug)]
pub(crate) enum OfficeAction {
    /// Print a page of the file's content with anchors, its structure, or
    /// its facts, as the reader projects it.
    Read {
        file: PathBuf,
        /// The unit to start the page at.
        #[arg(long, default_value_t = 0)]
        from: usize,
        /// Start the page at the unit this anchor names (`Budget!B4`,
        /// `slide:256`, `p:1A2B3C4D`).
        #[arg(long, conflicts_with = "from")]
        at: Option<String>,
        /// Parts, content types and relationships instead of content.
        #[arg(long, conflicts_with_all = ["at", "facts"])]
        structure: bool,
        /// Kind, counts and flags only.
        #[arg(long, conflicts_with = "at")]
        facts: bool,
    },
    /// Apply typed ops (the `office_apply` schema) and write the result to a
    /// new file. The source is never changed. With no source file and no
    /// digest, `--out` is created from scratch from the built-in blank.
    Apply {
        /// The file to start from; leave out, with --base-digest, to create
        /// from scratch.
        file: Option<PathBuf>,
        /// A JSON file holding the array of ops, or `-` for stdin.
        #[arg(long)]
        ops: String,
        /// The sha256 (or its first 16 hex digits) of the file the ops were
        /// written against, as `read` printed it. Required with a file.
        #[arg(long)]
        base_digest: Option<String>,
        /// Where to write the result; must not exist.
        #[arg(long)]
        out: PathBuf,
        /// The Agent id Word tracked changes are attributed to.
        #[arg(long, default_value = "vak")]
        agent: String,
    },
    /// The semantic change list from one file to another, with what the
    /// change does to signatures and sensitivity labels.
    Diff { before: PathBuf, after: PathBuf },
    /// Check that a file is a well-formed package of the format its name
    /// says. Exits 1 when it is not.
    Verify { file: PathBuf },
}

#[derive(Subcommand, Debug)]
pub(crate) enum IntentAction {
    /// Resolve a prompt and print the reading, every contributing signal, and
    /// the engagement diff against the unrestricted baseline. Costs nothing
    /// and dispatches nothing.
    Explain {
        prompt: String,
        /// Pretend the request arrived on this surface: cli, desktop, server,
        /// chat, cron, heartbeat, worker.
        #[arg(long)]
        surface: Option<String>,
        /// Override the reading's act.
        #[arg(long)]
        act: Option<String>,
        /// Override the reading's horizon.
        #[arg(long)]
        horizon: Option<String>,
        /// Override the reading's stakes.
        #[arg(long)]
        stakes: Option<String>,
        /// Override the reading's evidence standard.
        #[arg(long)]
        evidence: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Print the resolved intent policy for this workspace.
    Show,
}

#[derive(Subcommand, Debug)]
pub(crate) enum CommitAction {
    /// List commitments, scheduling order first.
    List {
        /// Include closed commitments.
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// Show one commitment: criteria, episodes, evidence, and closure.
    Show {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Close a commitment with an explicit verdict. A `fulfilled` claim is
    /// refused unless the recorded evidence actually supports it.
    Close {
        id: String,
        /// fulfilled | partial | failed | abandoned | expired | unknown
        #[arg(long, default_value = "abandoned")]
        verdict: String,
        #[arg(long, default_value = "")]
        note: String,
    },
    /// Replace one commitment with another, recording the lineage.
    Supersede {
        id: String,
        #[arg(long)]
        by: String,
        #[arg(long, default_value = "")]
        reason: String,
    },
    /// Record a human attestation against a criterion. The only way, other
    /// than an external receipt, to reach `attested` evidence.
    Attest {
        id: String,
        #[arg(long)]
        criterion: String,
        #[arg(long, default_value = "")]
        note: String,
    },
}
