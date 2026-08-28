# 36 — First-run onboarding and time-to-value

Status: proposed design, no implementation yet.

## Objective

A new user should reach one useful, audited agent result without needing to
understand vak's internal architecture.

Primary success target:

> From opening an installed build, a user with a provider credential completes
> a read-only workspace task and opens its dispatch receipt in under five
> minutes.

Secondary target:

> A user without a provider credential can inspect the product and run a safe,
> deterministic demonstration without granting workspace access or spending
> tokens.

Setup speed means time-to-first-value, not model latency or task execution
speed.

## Current-state audit

The repository already contains most of the required primitives, but they are
not composed into one onboarding experience.

| Area | Current behavior | Problem |
|---|---|---|
| Distribution | `README.md` leads with cloning and compiling; release output is raw binaries plus checksums/feed | Contributor setup is presented as user setup; no signed installer, DMG, package, or one-command bootstrap |
| Installation | `vak self install` places components, then on a fresh machine creates a default workspace, syncs services, starts the gateway, and installs/loads desktop service state | Installation has activation side effects before the user chooses CLI, desktop, or always-on operation |
| CLI entry | Running `vak` prints help; normal `exec` and `plan` are headless paths | No active first-run CLI wizard, although doc 29 P3 says one exists |
| Desktop entry | Project picker opens a folder; choosing it implicitly trusts project config and `.env` | Folder selection and privileged workspace trust are collapsed into one decision |
| Provider setup | A dismissible `SetupCard` accepts provider/key; Settings separately discovers models and applies provider/model | Setup is split across surfaces; saving a provider does not visibly establish the complete route contract |
| Permissions | Three modes exist and desktop Settings explains them | Permission choice is outside first run; defaults are applied without a deliberate onboarding decision |
| Health | `vak doctor`, `/health`, provider inventory, model discovery, key persistence, sandbox reporting, and install verification exist | Readiness is available as facts but not represented as one setup state |
| First task | Desktop shows three powerful starter prompts | Two starters request modifications and are too aggressive for first trust; none teaches receipts or the permission boundary |
| Receipts | Desktop has Dispatch Forensics and server receipt endpoints | The differentiator is hidden behind an icon after the task |
| Measurement | No onboarding milestones or time-to-value record | Regressions in activation friction cannot be detected |

Two documentation inconsistencies should be fixed as part of this work:

1. `docs/design/29-personal-os.md` says the CLI first-run wizard is complete,
   but no current CLI wizard path exists.
2. The README version badge/status can drift from the workspace version.

## Product decision

Desktop is the primary onboarding surface for end users. `vak setup` provides
the equivalent terminal flow for headless and developer installations. Both
consume the same backend readiness model and perform the same writes.

Installation and activation are separate contracts:

- **Install** places and verifies signed components. It does not start durable
  services or choose a workspace.
- **Setup** selects a workspace, resolves trust, connects a provider, freezes a
  default route, chooses a permission posture, checks execution readiness, and
  optionally activates durable services.
- **Run** remains fail-closed. Non-interactive commands never infer trust or
  consent from an unfinished setup.

This changes the current fresh-install bootstrap behavior deliberately. Service
activation remains available, but occurs only after an explicit "always-on"
choice and retains doc 32's workspace-identity invariant.

## User journey

### Desktop flow

The first launch is a resumable five-step flow.

#### 1. Welcome and usage mode

Ask what the user wants to do:

- **Use the desktop app** — recommended; no durable gateway service required.
- **Use the CLI** — show the installed command path and continue setup here.
- **Run an always-on assistant** — configure services after core readiness.

This choice controls which later steps appear; it does not alter permissions.

#### 2. Select and review workspace

The user chooses a directory. vak scans only for onboarding-relevant facts:

- Git repository status.
- Presence of `.vak/config.toml`.
- Presence of project `.env`.
- Privileged settings requested by the project: hooks, MCP, provider endpoint,
  sandbox, gateway, permission grants, and allow rules.

If neither privileged config nor `.env` exists, the workspace can open without
an additional trust prompt. If either exists, show a dedicated trust review:

> This project contains configuration that can run commands or redirect API
> traffic. Trusting it enables the listed capabilities.

Choices:

- **Open safely** — privileged project settings and project `.env` stay ignored.
- **Trust this workspace** — persist the canonical-path trust decision.
- **Cancel**.

Selecting a folder alone must no longer imply privileged trust.

#### 3. Connect model and choose route

Show providers using the existing provider inventory. Configured providers are
marked. Ollama is shown as local/keyless when reachable.

The sequence is transactional from the user's perspective:

1. Select provider.
2. Enter credential if required.
3. Verify authentication by discovering models.
4. Choose a discovered model, with an advanced exact-ID escape hatch.
5. Save provider and model atomically as the workspace default route.

A key being stored is not considered success until authentication/model
discovery succeeds. If discovery is unsupported or temporarily unavailable,
the UI may accept an exact model ID but labels route verification as incomplete.

Secrets continue to use the canonical user `.env`, owner-only permissions,
atomic replacement, and never appear in onboarding state or logs.

#### 4. Choose safety posture

Present three outcome-oriented presets backed by existing permission modes:

- **Inspect only** (`read-only`) — read and search inside the workspace; no
  modifications.
- **Code with approval** (`workspace-write`) — edit inside the workspace; ask
  before shell and sensitive actions. Recommended.
- **Unrestricted** (`full-access`) — host access; advanced and visually
  separated with explicit confirmation.

Show the effective sandbox beside the choice. If the selected restricted mode
has no supported containment backend, fail closed and explain the exact remedy.

Do not introduce a fourth permission mode merely for onboarding. Presets are
plain-language projections of the existing contract.

#### 5. Readiness and first result

Run a focused readiness check assembled from existing health capabilities:

- Workspace readable and sessions home writable.
- Trust decision recorded or safe mode active.
- Effective provider authenticated.
- Provider/model route verified or explicitly accepted as unverified.
- Effective permission mode persisted.
- Effective sandbox identified.
- Installed components match their manifest when a managed install exists.

On success, offer one recommended first task:

> Map this codebase and explain its architecture, key flows, and highest-risk
> areas. Do not modify files or run destructive commands.

The first task runs under an onboarding-scoped read-only cap even when the user
selected workspace-write. Completion opens a success sheet showing:

- Files/tools observed.
- Permission decisions.
- Provider and model.
- Token and estimated cost data when available.
- A prominent **Open receipt** action.

The user may skip the first task without losing configuration.

### CLI flow

`vak setup` implements the same state machine with terminal prompts. It is
explicitly invoked; running `vak exec` in scripts remains non-interactive and
fail-closed.

Expected flow:

```text
$ vak setup
Workspace: /work/project
Project configuration: privileged settings detected
Open safely / Trust / Cancel: Open safely
Provider: OpenAI Responses
Credential: [hidden]
Model: <discovered selection>
Safety: Code with approval
✓ provider authenticated
✓ route saved
✓ workspace boundary active
✓ install verified

Run the read-only starter task now? [Y/n]
```

CLI requirements:

- TTY-only prompts.
- `--non-interactive` accepts explicit flags and fails on any missing choice.
- `--json` returns the readiness state without prompts.
- No credential in argv. Non-interactive credentials come from environment or
  stdin through an explicit secret-input flag.
- Re-running `vak setup` resumes from actual state and permits changing any
  decision.

### No-key demonstration

`vak demo` runs a bundled deterministic scenario in a temporary scratch
workspace with a mock provider. It demonstrates a tool call, a denied action,
and a receipt without network access, credentials, services, or changes to the
selected repository.

The demo is educational, not readiness: it never marks a provider connected or
a real workspace trusted.

## Shared readiness model

Do not make a marker file the source of truth. Readiness is derived from actual
state so deleted credentials, moved workspaces, config drift, and revoked
permissions immediately make setup incomplete again.

Introduce a shared projection in `vak-core`, exposed by server and CLI:

```rust
pub struct OnboardingStatus {
    pub install: StepStatus,
    pub workspace: WorkspaceStatus,
    pub trust: TrustStatus,
    pub provider: ProviderStatus,
    pub route: RouteStatus,
    pub permission: PermissionStatus,
    pub sandbox: SandboxStatus,
    pub services: ServiceStatus,
    pub first_result: FirstResultStatus,
    pub core_ready: bool,
    pub unattended_ready: bool,
}
```

`core_ready` requires workspace, provider/route, permission, and writable
session storage. `unattended_ready` additionally requires service health,
channel policy, budget configuration acknowledgment, and no unresolved Ask path
that would silently auto-deny essential work.

Persist only user-experience metadata in
`<data_home>/onboarding.json`:

- Schema version.
- Last completed UI step.
- Welcome dismissed.
- First-result walkthrough completed/skipped.
- Selected usage mode.
- Local milestone timestamps when measurement is enabled.

Never persist credentials, trust as authority, permission grants, provider/model
authority, or service state in this file.

## API and command design

Reuse existing provider, model, config, health, receipts, and operations APIs.
Add only the composition layer:

- `GET /onboarding` — derived status plus presentation metadata.
- `PATCH /onboarding/preferences` — usage mode and walkthrough state only.
- `POST /onboarding/workspace-review` — inspect a path and report privileged
  config without activating it.
- `POST /onboarding/trust` — explicit canonical-path trust or safe-open choice.
- `POST /onboarding/verify-route` — authenticated model discovery/verification.
- `POST /onboarding/first-task` — read-only capped starter run.
- `vak setup` and `vak setup status [--json]`.
- `vak demo`.

Provider and model must continue to be written atomically as one route.
Permission changes must keep the existing revoke-before-apply behavior.

## Error design

Every setup failure has four fields:

1. What failed.
2. What state is still safe/preserved.
3. One primary repair action.
4. A details disclosure containing the original typed error.

Example:

```text
Could not authenticate OpenAI Responses.
Your key was stored on this device, but no route was activated.

Check the key and try again. Existing workspace permissions were unchanged.
```

The UI must not collapse provider authentication, model discovery, sandbox
availability, and general backend boot into one "setup failed" message.

## Distribution design

Onboarding cannot meet its target while compilation is the primary install
path.

Required release artifacts:

- macOS signed and notarized application bundle, distributed as DMG or zipped
  app bundle.
- Linux versioned archive containing CLI and workers, with a verified installer
  script initially; native packages can follow.
- Checksums and provenance generated by the existing release pipeline.
- Raw binaries retained for advanced users and updates.

The README order becomes:

1. Download/install release.
2. Launch desktop or run `vak setup`.
3. Complete first audited task.
4. Build from source under Development.

Version text in README should derive during release or be removed; the workspace
manifest remains the sole authoritative version.

## Measurement and privacy

Record onboarding milestones locally by default, with no network transmission:

- `install_seen_at`.
- `workspace_selected_at`.
- `provider_verified_at`.
- `core_ready_at`.
- `first_run_started_at`.
- `first_result_at`.
- Failure category counts, never secret values or paths.

Expose `vak setup status --timings` and a "Copy setup diagnostics" action. Any
future remote analytics must be a separate explicit opt-in design.

Success measures:

| Measure | Target |
|---|---:|
| Installed app to workspace selection | under 60 seconds |
| Workspace selection to provider verification | under 2 minutes when key is available |
| Installed app to core ready | under 3 minutes |
| Installed app to first audited result | under 5 minutes |
| Re-entry after an interrupted setup | resumes at the first incomplete step |
| Unexplained setup dead ends | zero in usability tests |

## Security invariants

1. Installation never grants workspace trust or starts unattended agents.
2. Folder selection is not consent to privileged project configuration.
3. Non-interactive paths never prompt or infer consent.
4. Credentials are accepted once, stored only through the canonical secret
   path, and never copied into onboarding state, logs, process arguments, or
   receipts.
5. The first guided task is capped to read-only regardless of the configured
   workspace mode.
6. `full-access` requires a separate explicit confirmation and is never the
   recommended preset.
7. Setup uses the same permission engine, route writes, provider registry,
   sandbox resolution, and service manager as ordinary operation.
8. A stale onboarding preference can alter presentation only; it cannot grant
   capability.
9. Durable services retain the workspace identity captured at explicit
   activation.
10. Demo mode is isolated from real workspaces and cannot consume real provider
    credentials.

## Implementation plan

### Phase O0 — Contract correction and baseline

Deliver:

- Update doc 29 to mark the missing CLI wizard honestly.
- Remove README version duplication and make current user/contributor paths
  explicit.
- Add an onboarding test fixture that starts with a clean data home.
- Capture baseline manual timings for source-build desktop and CLI flows.

Exit criteria:

- Documentation matches executable behavior.
- A clean-state test can assert current onboarding facts without touching the
  operator's real data home.

### Phase O1 — Shared readiness projection

Deliver:

- `OnboardingStatus` in `vak-core` derived from current authority sources.
- Read-only server endpoint and `vak setup status --json`.
- Typed failure categories for provider, route, trust, sandbox, install, and
  service readiness.
- Presentation-only onboarding metadata store.

Exit criteria:

- CLI and desktop receive byte-equivalent readiness facts for the same clean
  environment.
- Removing a provider key or moving a workspace immediately invalidates the
  corresponding readiness step without editing onboarding metadata.

### Phase O2 — Separate installation from activation

Deliver:

- `self install` places and verifies components only.
- Move default workspace creation, service sync/start, and tray activation into
  explicit setup actions.
- Preserve existing installs and service configurations during upgrades.
- Release artifacts suitable for installation without a Rust toolchain.

Exit criteria:

- Fresh install starts no gateway, Telegram, or unattended agent service.
- Upgrade preserves existing service bindings and data.
- Uninstall remains symmetric and manifest-driven.

### Phase O3 — Desktop onboarding

Deliver:

- Five-step resumable flow.
- Workspace privileged-config review and explicit trust/safe-open decision.
- Provider authentication plus live model discovery and atomic route save.
- Permission presets and effective sandbox explanation.
- Readiness summary with targeted repairs.

Exit criteria:

- A clean user can become core-ready without opening Settings or editing a
  file.
- Closing the app after any step resumes correctly.
- No provider credential leaves the secret API boundary.

### Phase O4 — CLI onboarding

Deliver:

- `vak setup`, status, JSON, and explicit non-interactive mode.
- Hidden credential input and environment/stdin automation path.
- The same trust review, route verification, permission selection, and
  readiness rendering as desktop.

Exit criteria:

- Desktop and CLI create identical effective config from equivalent choices.
- Piped/non-TTY setup fails closed unless every required choice is explicit.

### Phase O5 — First-result walkthrough and demo

Deliver:

- Read-only first-task endpoint and desktop/CLI walkthrough.
- Completion summary with one-click/one-command receipt access.
- Isolated deterministic `vak demo` scenario.
- Replace aggressive first-run starter prompts until the walkthrough is
  complete.

Exit criteria:

- First guided run cannot modify the selected workspace even if the workspace
  default is full-access.
- Demo performs no network calls, writes only to its temporary workspace, and
  leaves a complete demonstrative receipt.

### Phase O6 — Release UX and usability gate

Deliver:

- Signed/notarized macOS artifact and verified Linux install path.
- Release landing/README flow centered on setup, not compilation.
- Local milestone timings and copyable setup diagnostics.
- Moderated clean-machine tests with at least five people unfamiliar with vak.

Exit criteria:

- Median time to core-ready is under three minutes for users with a key.
- Median time to first audited result is under five minutes.
- Every observed failure has a visible recovery action.
- Release verification, workspace tests, and clean-machine smoke tests pass.

## Test plan

### Deterministic tests

- Readiness derivation across every missing/present state combination.
- Trust review detects each privileged project section without loading it.
- Safe-open cannot load project `.env`, hooks, MCP, base URLs, or grants.
- Provider key save followed by failed discovery leaves route unchanged.
- Successful provider/model apply is atomic across concurrent readers.
- Permission apply revokes active capabilities before changing mode.
- First task is read-only capped in all three configured modes.
- Onboarding metadata corruption cannot grant capability.
- Re-running setup is idempotent.
- Non-TTY setup with missing choices refuses rather than assuming.
- Fresh install starts no durable service; explicit always-on setup captures the
  selected workspace.

### End-to-end tests

- Clean macOS app launch to first receipt.
- Clean Linux CLI install to first receipt.
- Ollama/keyless path.
- Invalid key, valid key with model-discovery outage, and provider outage.
- Workspace with hostile privileged config opened safely.
- Interrupted setup resume after every step.
- Upgrade from the current auto-service install without losing service state.
- Demo with network disabled and real provider variables present.

## Non-goals

- Adding providers or messaging channels.
- Redesigning the permission engine.
- Replacing doctor or provider/model APIs.
- Automatically enabling full access.
- Automatically trusting a selected repository.
- Sending onboarding analytics remotely.
- Making unattended gateway setup part of the five-minute core path.

## Open decisions before implementation

1. Exact macOS distribution format: DMG versus signed zipped app bundle.
2. Whether Linux starts with a verified shell installer or native packages.
3. Whether `vak demo` ships inside the production binary or as a bundled
   scenario asset.
4. Whether existing fresh-install service bootstrap is removed immediately or
   retained for one release behind an explicit compatibility flag.

These choices affect packaging and migration, not the readiness or security
contracts above.
