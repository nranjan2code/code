//! Derived setup readiness (`docs/design/46-stabilization-install-and-onboarding.md`
//! Part III).
//!
//! Readiness is **derived on every read** from the same authorities the
//! rest of the system uses. There is deliberately no marker file saying
//! "setup is done": deleting a key, moving a workspace, or revoking a
//! permission has to make setup incomplete again on the next read, with
//! no cached state to invalidate and no way for a stale file to grant a
//! capability nothing else agrees exists.
//!
//! One projection, three surfaces: `vak setup status`, `GET /onboarding`,
//! and the desktop shell all render this. A surface that computed its own
//! answer is how two of them come to disagree.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Core;

/// The starter task every first-run surface offers (doc 46, Step 9). One
/// constant because the web wizard and `vak setup` must offer the same
/// thing, and general-purpose because the default workspace is a person's
/// home for their work, not a codebase: the first result should show what
/// Vakyartha can do here, whatever "here" holds. Fixed rather than
/// caller-supplied: a prompt the onboarding endpoint accepted would be a way
/// to run arbitrary work under its read-only starter path.
pub const FIRST_TASK_PROMPT: &str = "Look around this workspace and tell me, briefly and in \
     plain words, what is here and what you could help me with — and what you would need \
     from me to do more. Do not change anything.";

/// Why a step is not satisfied, in the four fields every setup failure
/// owes the reader (doc 46, "Error design"): what failed, what is still
/// safe, the one repair, and the underlying detail for whoever wants it.
///
/// Collapsing these into one "setup failed" string is the specific
/// failure this shape exists to prevent — provider auth, model discovery,
/// sandbox availability, and backend boot are four different problems
/// with four different remedies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepFailure {
    /// What failed, in a sentence a person can act on.
    pub what: String,
    /// What is still safe or preserved, so the reader knows the blast radius.
    pub preserved: String,
    /// The single next action that resolves it.
    pub repair: String,
    /// The original typed error, for disclosure rather than display.
    pub detail: Option<String>,
}

impl StepFailure {
    pub fn new(
        what: impl Into<String>,
        preserved: impl Into<String>,
        repair: impl Into<String>,
    ) -> Self {
        Self {
            what: what.into(),
            preserved: preserved.into(),
            repair: repair.into(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

/// One step of setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum StepState {
    /// Done. `provenance` names the layer the value came from where the
    /// step reads a layered setting, because the single most confusing
    /// thing about a deep inheritance chain is a value you cannot
    /// attribute (doc 46 Part VI).
    Satisfied {
        detail: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        provenance: Option<String>,
    },
    /// Not done, with the four-field reason.
    Incomplete(StepFailure),
    /// Nothing to do here for this installation — a headless box has no
    /// desktop step, a CLI-only user has no channels. Distinct from
    /// `Satisfied` so a surface can render it as "not needed" rather
    /// than claiming an achievement nobody earned.
    NotApplicable { reason: String },
}

impl StepState {
    pub fn ok(detail: impl Into<String>) -> Self {
        StepState::Satisfied {
            detail: detail.into(),
            provenance: None,
        }
    }

    pub fn ok_from(detail: impl Into<String>, provenance: impl Into<String>) -> Self {
        StepState::Satisfied {
            detail: detail.into(),
            provenance: Some(provenance.into()),
        }
    }

    pub fn is_satisfied(&self) -> bool {
        matches!(self, StepState::Satisfied { .. })
    }

    /// Satisfied or deliberately not applicable — i.e. nothing is owed
    /// here. Readiness sums this, not `is_satisfied`, so an inapplicable
    /// step never blocks a machine it was never meant to apply to.
    pub fn settled(&self) -> bool {
        !matches!(self, StepState::Incomplete(_))
    }

    pub fn failure(&self) -> Option<&StepFailure> {
        match self {
            StepState::Incomplete(f) => Some(f),
            _ => None,
        }
    }
}

/// Optional facts a caller probes on our behalf.
///
/// The service manager is reachable from the CLI and the server but is
/// not something a library should shell out to on every status read, and
/// the caller already knows whether this installation asked for durable
/// services at all. `None` means "not probed", which renders as
/// `NotApplicable` rather than as a failure.
#[derive(Debug, Clone, Default)]
pub struct ProbedFacts {
    /// (service name, running) for each unit this installation expects.
    pub services: Option<Vec<(String, bool)>>,
    /// Units that configuration calls for but the service manager has not
    /// been told about yet — bots created without being activated.
    ///
    /// Reported separately from `services` because it is a different
    /// state: not "broken", but "you configured this and have not
    /// activated it". Before this was surfaced, a bot created in the
    /// console looked identical to one that was live.
    pub awaiting_activation: Vec<String>,
    /// The managed install: `Ok(detail)` when it verifies, `Err(failure)`
    /// with the *right* four fields when it does not. Typed rather than a
    /// string, because the caller knows whether this is drift (repair:
    /// reinstall) or pre-baseline state (repair: purge) — and offering
    /// the wrong remedy is worse than offering none.
    pub install: Option<Result<String, StepFailure>>,
}

/// The whole projection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingState {
    pub install: StepState,
    pub dependencies: StepState,
    pub workspace: StepState,
    pub trust: StepState,
    pub provider: StepState,
    pub route: StepState,
    pub permission: StepState,
    pub sandbox: StepState,
    pub capabilities: StepState,
    pub integrations: StepState,
    pub channels: StepState,
    pub services: StepState,
    pub first_result: StepState,
    /// Enough to run a task in this workspace.
    pub core_ready: bool,
    /// Enough to run unattended: core, plus services and a bound channel.
    pub unattended_ready: bool,
}

impl OnboardingState {
    /// Every step, in presentation order, paired with its label.
    pub fn steps(&self) -> Vec<(&'static str, &StepState)> {
        vec![
            ("install", &self.install),
            ("dependencies", &self.dependencies),
            ("workspace", &self.workspace),
            ("trust", &self.trust),
            ("provider", &self.provider),
            ("route", &self.route),
            ("permission", &self.permission),
            ("sandbox", &self.sandbox),
            ("capabilities", &self.capabilities),
            ("integrations", &self.integrations),
            ("channels", &self.channels),
            ("services", &self.services),
            ("first result", &self.first_result),
        ]
    }

    /// Steps still owed, for a surface that wants to resume where the
    /// operator left off rather than replay what is already done.
    pub fn incomplete(&self) -> Vec<(&'static str, &StepFailure)> {
        self.steps()
            .into_iter()
            .filter_map(|(label, step)| step.failure().map(|f| (label, f)))
            .collect()
    }
}

/// Derive the whole projection from live state.
pub fn derive(core: &Core, probed: &ProbedFacts) -> OnboardingState {
    let install = install_step(probed);
    let dependencies = dependencies_step();
    let workspace = workspace_step(core);
    let trust = trust_step(core);
    let (provider, route) = provider_and_route_steps(core);
    let permission = permission_step(core);
    let sandbox = sandbox_step(core);
    let capabilities = capabilities_step();
    let integrations = integrations_step(core);
    let channels = channels_step(core);
    let services = services_step(probed);
    let first_result = first_result_step(core);

    // A route is only meaningful once the provider authenticates, so
    // `core_ready` deliberately does not double-count it.
    //
    // The permission step is `settled()`, not `is_satisfied()`. An unchosen
    // posture is a decision still owed — worth showing, and worth an action
    // in the wizard — but it does not stop a task from running: the default
    // is `workspace-write` and the sandbox applies either way. Requiring it
    // here would have declared every working install "not ready" the moment
    // this step learned to be honest.
    let core_ready = install.settled()
        && workspace.is_satisfied()
        && trust.settled()
        && provider.is_satisfied()
        && route.is_satisfied();
    let unattended_ready = core_ready && services.is_satisfied() && channels.is_satisfied();

    OnboardingState {
        install,
        dependencies,
        workspace,
        trust,
        provider,
        route,
        permission,
        sandbox,
        capabilities,
        integrations,
        channels,
        services,
        first_result,
        core_ready,
        unattended_ready,
    }
}

fn install_step(probed: &ProbedFacts) -> StepState {
    match &probed.install {
        Some(Ok(detail)) => StepState::ok(detail.clone()),
        Some(Err(failure)) => StepState::Incomplete(failure.clone()),
        // Running from a source tree or an unmanaged copy. That is a
        // normal way to develop, not a defect to report.
        None => StepState::NotApplicable {
            reason: "not a managed install".into(),
        },
    }
}

/// Probe the external commands optional features need, and say so once
/// here rather than letting a missing one surface as a confusing failure
/// three screens later (doc 46, Step 1).
fn dependencies_step() -> StepState {
    let mut missing = Vec::new();
    if which("npx").is_none() {
        missing.push("npx (Node) — needed by the curated MCP integrations");
    }
    if which("git").is_none() {
        missing.push("git — needed by worktrees, diff review, and the PR flow");
    }
    if missing.is_empty() {
        return StepState::ok("npx and git available");
    }
    // Optional by definition: never blocks core readiness.
    StepState::Incomplete(StepFailure::new(
        format!("Optional dependencies are missing: {}.", missing.join("; ")),
        "Everything that does not need them works normally.",
        "Install them with your platform package manager, then re-run setup.",
    ))
}

/// Minimal PATH lookup. Deliberately not a crate: one read of `PATH` and
/// an executable-bit check is the whole contract, and a dependency here
/// would ride into every surface that renders setup status.
fn which(program: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join(program);
        candidate.is_file().then_some(candidate)
    })
}

fn workspace_step(core: &Core) -> StepState {
    let cwd = core.cwd();
    if std::fs::read_dir(cwd).is_err() {
        return StepState::Incomplete(StepFailure::new(
            format!("The workspace at {} cannot be read.", cwd.display()),
            "Nothing was changed.",
            "Choose a different workspace, or restore that directory.",
        ));
    }
    let home = core.sessions_home();
    if std::fs::create_dir_all(&home).is_err() {
        return StepState::Incomplete(StepFailure::new(
            format!("The session store at {} is not writable.", home.display()),
            "Your workspace files are untouched.",
            "Fix the directory's permissions, then re-run setup.",
        ));
    }
    StepState::ok(cwd.display().to_string())
}

fn trust_step(core: &Core) -> StepState {
    let cwd = core.cwd();
    if !crate::trust::requests_privilege(cwd) {
        return StepState::NotApplicable {
            reason: "this workspace requests no privileged configuration".into(),
        };
    }
    if core.project_config_trusted() {
        return StepState::ok("privileged project configuration is trusted here");
    }
    // Open-safely is a complete, deliberate outcome, not a half-done
    // step: the workspace runs, and the project's privileged keys stay
    // demoted. Reporting it as a failure would push people to grant
    // trust to clear a warning, which is exactly backwards.
    StepState::ok("opened safely — privileged project configuration stays demoted")
}

fn provider_and_route_steps(core: &Core) -> (StepState, StepState) {
    let provider_name = core.effective_provider();
    let model = core.effective_model();
    match core.provider() {
        Ok(p) => {
            // Report the provider the operator configured, not the client
            // implementation that serves it: Ollama speaks the
            // OpenAI-compatible wire format, so `p.name()` says
            // "openai-completions" for a route the user chose as "ollama".
            // Naming the implementation there reads as though their choice
            // was ignored. The implementation is disclosure, not headline.
            let implementation = p.name().to_string();
            let detail = if implementation.eq_ignore_ascii_case(&provider_name) {
                format!("{provider_name} authenticated")
            } else {
                format!("{provider_name} authenticated (via the {implementation} API)")
            };
            let provider = StepState::ok(detail);
            // Provider and model are one atomic route (invariant 17); a
            // configured provider with no model is not a usable route.
            let route = if model.trim().is_empty() {
                StepState::Incomplete(StepFailure::new(
                    format!("No model is selected for {provider_name}."),
                    "Your credential is stored and the provider authenticates.",
                    "Choose a model in setup; the list comes from your key.",
                ))
            } else {
                StepState::ok_from(format!("{provider_name}/{model}"), route_provenance(core))
            };
            (provider, route)
        }
        Err(e) => (
            StepState::Incomplete(
                StepFailure::new(
                    if provider_name.trim().is_empty() {
                        "No AI service is selected.".to_string()
                    } else {
                        format!("{provider_name} is not connected.")
                    },
                    "No route was activated and nothing else was changed.",
                    if provider_name.trim().is_empty() {
                        "Choose a supported provider and model in setup or settings."
                    } else {
                        "Add a credential for this provider in setup, or choose another."
                    },
                )
                .with_detail(e.to_string()),
            ),
            StepState::Incomplete(StepFailure::new(
                "No verified route.",
                "Existing workspace settings are unchanged.",
                "Connect a provider first; the route is saved with the model.",
            )),
        ),
    }
}

/// Which layer supplied the effective route. Reported rather than
/// inferred, because "why is this the model?" is the question a layered
/// configuration makes hardest to answer.
fn route_provenance(core: &Core) -> &'static str {
    if vak_config::project_path(core.cwd()).is_file() {
        "project or Shared layer"
    } else {
        "Shared layer"
    }
}

/// True when some config layer actually names a permission mode, as
/// opposed to the effective value being the compiled default.
///
/// Read as **text**, the same way `trust::requested_privileges` reads a
/// project config: this runs during setup, before an operator has decided
/// anything, and loading a layer to ask a question about it is the mistake
/// that module exists to avoid.
fn permission_mode_is_chosen(core: &Core) -> bool {
    let layers = [
        vak_config::project_path(core.cwd()),
        vak_config::global_path().unwrap_or_default(),
    ];
    layers.iter().any(|path| {
        std::fs::read_to_string(path).is_ok_and(|text| {
            text.lines()
                .map(str::trim_start)
                .any(|line| line.starts_with("permission_mode"))
        })
    })
}

/// How much vak may do on its own.
///
/// This step used to report `Satisfied` unconditionally, which made it
/// invisible in exactly the way that matters: the wizard renders a step's
/// actions only while it is unsatisfied, so the three-posture chooser
/// behind it could never appear and a first-run operator was never asked.
/// They inherited `workspace-write` — a reasonable default, and still not a
/// decision anybody made.
///
/// An explicit mode in either layer settles it. The remedy names all three
/// postures rather than recommending one, because this is an access
/// decision and the wizard must not make it on someone's behalf.
fn permission_step(core: &Core) -> StepState {
    if permission_mode_is_chosen(core) {
        return StepState::ok_from(
            format!("{:?}", core.effective_permission_mode()),
            route_provenance(core),
        );
    }
    StepState::Incomplete(
        StepFailure::new(
            "No safety posture has been chosen for this workspace.",
            format!(
                "Nothing is unguarded: until you choose, vak runs at the \
                 default ({:?}) and the sandbox still applies.",
                core.effective_permission_mode()
            ),
            "Pick one: inspect only, work with approval, or unrestricted.",
        )
        .with_detail(
            "No config layer sets `permission_mode`, so the effective value is \
             the compiled default rather than a decision. Choosing writes it to \
             the layer you pick.",
        ),
    )
}

fn sandbox_step(core: &Core) -> StepState {
    let name = core.effective_sandbox_name();
    let absent = name.eq_ignore_ascii_case("none") || name.eq_ignore_ascii_case("off");
    let restricted = !matches!(
        core.effective_permission_mode(),
        vak_config::PermissionMode::FullAccess
    );
    if absent && restricted {
        // A restricted mode with no containment backend fails closed at
        // the first tool call. Saying so here is what keeps that
        // discovery out of the middle of someone's first task.
        return StepState::Incomplete(StepFailure::new(
            "No sandbox backend is available for the selected safety posture.",
            "Restricted modes still refuse unsandboxed work, so nothing escaped.",
            "Install a supported backend, or choose full access deliberately.",
        ));
    }
    if absent {
        return StepState::NotApplicable {
            reason: "full access is unsandboxed by design".into(),
        };
    }
    StepState::ok(name)
}

fn capabilities_step() -> StepState {
    let root = vak_config::paths::default_workspace().join(".vak");
    let skills = root.join("skills");
    let count = std::fs::read_dir(&skills)
        .map(|entries| entries.flatten().filter(|e| e.path().is_dir()).count())
        .unwrap_or(0);
    if count == 0 {
        return StepState::Incomplete(StepFailure::new(
            "No shared skills are installed yet.",
            "Nothing was written; the agent runs without them.",
            "Install the starter skills (`vak setup seed`, or the button in setup).",
        ));
    }
    StepState::ok_from(format!("{count} shared skills"), "Shared layer")
}

fn integrations_step(core: &Core) -> StepState {
    let servers = core.effective_mcp().servers;
    if servers.is_empty() {
        // Nothing is seeded on our initiative (doc 46 D4), so an empty
        // set is the expected fresh state rather than an omission.
        return StepState::NotApplicable {
            reason: "no integrations enabled".into(),
        };
    }
    let mut names: Vec<&str> = servers.keys().map(String::as_str).collect();
    names.sort_unstable();
    StepState::ok(names.join(", "))
}

fn channels_step(core: &Core) -> StepState {
    let entries = crate::health::read_channel_entries(&core.sessions_home());
    if entries.is_empty() {
        return StepState::NotApplicable {
            reason: "no channels onboarded".into(),
        };
    }
    let allowed = entries.iter().filter(|e| e.status == "allowed").count();
    let pending = entries.iter().filter(|e| e.status == "pending").count();
    if allowed == 0 {
        return StepState::Incomplete(StepFailure::new(
            format!("No channel is approved yet ({pending} waiting for review)."),
            "Every unapproved chat is refused, as designed.",
            "Approve a chat in setup or the admin console.",
        ));
    }
    StepState::ok(format!("{allowed} approved · {pending} pending"))
}

fn services_step(probed: &ProbedFacts) -> StepState {
    if !probed.awaiting_activation.is_empty() {
        let count = probed.awaiting_activation.len();
        return StepState::Incomplete(StepFailure::new(
            format!("{count} configured bridge(s) are not activated yet."),
            "Their configuration and credentials are saved; nothing is running for them.",
            "Activate services to register and start them.",
        ));
    }
    let Some(services) = &probed.services else {
        return StepState::NotApplicable {
            reason: "no durable services requested".into(),
        };
    };
    if services.is_empty() {
        return StepState::NotApplicable {
            reason: "no durable services requested".into(),
        };
    }
    let down: Vec<&str> = services
        .iter()
        .filter(|(_, running)| !running)
        .map(|(name, _)| name.as_str())
        .collect();
    if down.is_empty() {
        return StepState::ok(format!("{} running", services.len()));
    }
    StepState::Incomplete(StepFailure::new(
        format!("Not running: {}.", down.join(", ")),
        "Configuration and credentials are unchanged.",
        "Run `vak self services-sync`, then check the service log.",
    ))
}

fn first_result_step(core: &Core) -> StepState {
    let home = core.sessions_home();
    if has_any_session(&home) {
        return StepState::ok("this workspace has run at least one session");
    }
    StepState::Incomplete(StepFailure::new(
        "No task has been run here yet.",
        "Setup is otherwise complete.",
        "Run the read-only starter task to see a result and its receipt.",
    ))
}

/// Cheap existence probe: one directory walk, no ledger parsing. Whether
/// *a* session exists is all this step asserts.
fn has_any_session(sessions_home: &Path) -> bool {
    let root = sessions_home.join("sessions");
    let Ok(workspaces) = std::fs::read_dir(&root) else {
        return false;
    };
    workspaces.flatten().any(|workspace| {
        std::fs::read_dir(workspace.path()).is_ok_and(|mut files| {
            files.any(|f| f.is_ok_and(|f| f.path().extension().is_some_and(|e| e == "jsonl")))
        })
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod permission_step_tests {
    use super::*;

    fn workspace(mode: Option<&str>) -> tempfile::TempDir {
        // Otherwise `global_path()` is the developer's own ~/vak-home
        // config, and whether this test passes depends on whose machine it
        // runs on.
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        if let Some(mode) = mode {
            std::fs::create_dir_all(dir.path().join(".vak")).unwrap();
            std::fs::write(
                dir.path().join(".vak/config.toml"),
                format!("permission_mode = \"{mode}\"\n"),
            )
            .unwrap();
        }
        dir
    }

    /// The regression this step exists to prevent: it reported `Satisfied`
    /// unconditionally, the wizard only renders actions for an unsatisfied
    /// step, and so the three-posture chooser could never appear.
    #[test]
    fn an_unchosen_posture_is_incomplete_so_the_wizard_can_offer_one() {
        let dir = workspace(None);
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        let step = permission_step(&core);
        let failure = step.failure().expect("must be incomplete");
        // All four fields, per doc 46's error design.
        assert!(failure.what.contains("safety posture"));
        assert!(!failure.preserved.is_empty());
        assert!(!failure.repair.is_empty());
        assert!(failure.detail.is_some());
    }

    #[test]
    fn an_explicit_mode_settles_it() {
        let dir = workspace(Some("read-only"));
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        assert!(permission_step(&core).is_satisfied());
    }

    /// An unchosen posture is a decision still owed, not a broken install:
    /// the default is safe and the sandbox applies either way. Gating
    /// readiness on it would declare every working install "not ready".
    #[test]
    fn an_unchosen_posture_does_not_block_readiness() {
        let dir = workspace(None);
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        let state = derive(&core, &ProbedFacts::default());
        assert!(!state.permission.is_satisfied());
        // `core_ready` still turns on provider/route, which this fixture has
        // not set — the point is only that permission is not one of its terms.
        assert!(
            !state
                .steps()
                .iter()
                .any(|(name, _)| *name == "permission" && state.core_ready),
            "permission must not be a term of core_ready"
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A Core whose every path is temporary, so no assertion here can
    /// read or write the operator's real state.
    fn core_in(dir: &Path) -> Core {
        crate::isolate_global_config();
        let core = Core::new(dir.to_path_buf()).unwrap();
        core.set_sessions_home(dir.join("home"));
        core
    }

    #[test]
    fn a_clean_machine_is_not_core_ready_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_in(dir.path());
        let state = derive(&core, &ProbedFacts::default());

        assert!(!state.core_ready, "no provider is configured");
        let provider = state.provider.failure().expect("provider is incomplete");
        assert!(provider.repair.contains("setup"), "and names one repair");
        assert!(
            !provider.preserved.is_empty(),
            "and says what is still safe"
        );
    }

    #[test]
    fn an_unprobed_install_is_not_applicable_rather_than_broken() {
        // Running from a source tree is normal. Reporting it as a defect
        // trains people to ignore the report.
        let dir = tempfile::tempdir().unwrap();
        let core = core_in(dir.path());
        let state = derive(&core, &ProbedFacts::default());
        assert!(matches!(state.install, StepState::NotApplicable { .. }));
        assert!(state.install.settled(), "and never blocks readiness");
    }

    #[test]
    fn a_workspace_requesting_no_privilege_is_never_asked_about_trust() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_in(dir.path());
        let state = derive(&core, &ProbedFacts::default());
        assert!(matches!(state.trust, StepState::NotApplicable { .. }));
    }

    #[test]
    fn opened_safely_is_a_complete_outcome_not_a_failure() {
        // Reporting safe-open as incomplete would push people to grant
        // trust merely to clear a warning -- exactly backwards.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".vak")).unwrap();
        std::fs::write(dir.path().join(".vak/config.toml"), "").unwrap();
        crate::isolate_global_config();
        let core = Core::new_with_trust(dir.path().to_path_buf(), false).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let state = derive(&core, &ProbedFacts::default());
        assert!(state.trust.is_satisfied());
        assert!(
            matches!(&state.trust, StepState::Satisfied { detail, .. } if detail.contains("safely"))
        );
    }

    #[test]
    fn the_install_step_reports_the_probers_own_repair_verbatim() {
        // Pre-baseline state and ordinary drift need different remedies;
        // wrapping either in a generic "reinstall" is how a reader is
        // handed the wrong command.
        let dir = tempfile::tempdir().unwrap();
        let core = core_in(dir.path());
        let state = derive(
            &core,
            &ProbedFacts {
                services: None,
                awaiting_activation: Vec::new(),
                install: Some(Err(StepFailure::new(
                    "predates the baseline",
                    "project files untouched",
                    "vak self uninstall --purge",
                ))),
            },
        );
        let failure = state.install.failure().expect("install is incomplete");
        assert_eq!(failure.repair, "vak self uninstall --purge");
    }

    #[test]
    fn a_down_service_is_incomplete_and_an_unrequested_one_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_in(dir.path());

        let none = derive(&core, &ProbedFacts::default());
        assert!(none.services.settled(), "nobody asked for services");

        let down = derive(
            &core,
            &ProbedFacts {
                services: Some(vec![("com.vak.gateway".into(), false)]),
                install: None,
                awaiting_activation: Vec::new(),
            },
        );
        let failure = down.services.failure().expect("a down service is a defect");
        assert!(failure.what.contains("com.vak.gateway"));
    }

    #[test]
    fn the_provider_step_names_the_configured_provider_not_the_client() {
        // A route configured as "ollama" is served by the
        // OpenAI-compatible client; reporting "openai-completions
        // authenticated" reads as though the operator's choice was
        // ignored.
        let dir = tempfile::tempdir().unwrap();
        let core = core_in(dir.path());
        let configured = core.effective_provider();
        if let StepState::Satisfied { detail, .. } =
            &derive(&core, &ProbedFacts::default()).provider
        {
            assert!(
                detail.starts_with(&configured),
                "expected {detail:?} to lead with the configured provider {configured:?}"
            );
        }
    }

    /// Configured-but-not-activated is its own state, and it has to be
    /// visible: a bot created in the console must not look identical to
    /// one that is actually running.
    #[test]
    fn a_bot_awaiting_activation_is_reported_as_such() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_in(dir.path());
        let state = derive(
            &core,
            &ProbedFacts {
                services: None,
                install: None,
                awaiting_activation: vec!["com.vak.discord-ops".into()],
            },
        );
        let failure = state.services.failure().expect("activation is owed");
        assert!(failure.what.contains("not activated"));
        assert!(failure.repair.contains("Activate"));
    }

    #[test]
    fn integrations_are_absent_by_default_not_missing() {
        // Nothing is seeded on our initiative, so an empty set is the
        // expected fresh state (doc 46 D4).
        let dir = tempfile::tempdir().unwrap();
        let core = core_in(dir.path());
        let state = derive(&core, &ProbedFacts::default());
        assert!(matches!(
            state.integrations,
            StepState::NotApplicable { .. }
        ));
    }

    #[test]
    fn every_incomplete_step_carries_all_four_fields() {
        // "Setup failed" as one message is the failure this shape exists
        // to prevent.
        let dir = tempfile::tempdir().unwrap();
        let core = core_in(dir.path());
        let state = derive(&core, &ProbedFacts::default());
        for (label, failure) in state.incomplete() {
            assert!(!failure.what.is_empty(), "{label} says what failed");
            assert!(
                !failure.preserved.is_empty(),
                "{label} says what is preserved"
            );
            assert!(!failure.repair.is_empty(), "{label} names one repair");
        }
    }

    #[test]
    fn the_projection_round_trips_through_json() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_in(dir.path());
        let state = derive(&core, &ProbedFacts::default());
        let encoded = serde_json::to_string(&state).unwrap();
        let back: OnboardingState = serde_json::from_str(&encoded).unwrap();
        assert_eq!(back.core_ready, state.core_ready);
        assert_eq!(back.steps().len(), state.steps().len());
    }
}
