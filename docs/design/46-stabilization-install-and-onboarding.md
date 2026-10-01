# 46 — Stabilization: one bundle, one install, one first run

Status: **historical proposal, superseded by the shipped 2.0.0 baseline and
3.0.10 lifecycle hardening**. Supersedes `docs/design/36-first-run-onboarding.md` and
`docs/design/37-distribution.md`, which are correct in analysis and stalled
in delivery; deletes `docs/design/35-tavily-integration.md`, whose premise
(one integration is "first-class") is withdrawn. `docs/design/32-release-engineering.md`
keeps the installed lifecycle contract and is amended, not replaced.

This document preserves the stabilization analysis and acceptance criteria
that led to the shipped baseline. Its dated audit below is historical; current
behavior is owned by `docs/design/00-roadmap.md`,
`docs/design/32-release-engineering.md`, and the current implementation.

## Scope discipline

Four things are being stabilized, in this dependency order:

1. **Bundle** — what a release produces.
2. **Release** — who produces it and what evidence rides along.
3. **Install** — how bits land, and how they leave without residue.
4. **Setup** — the one first run that makes a fresh machine useful.

Anything that is not one of those four is out of scope for this doc, even
where it is adjacent and tempting (new providers, new channels, permission
engine changes, Windows).

---

## Part I — What is actually true today

Audited 2026-09-02 against `main` at 1.0.3. Every row was read, not inferred.

### Bundling

| Fact | Evidence |
|---|---|
| No signing, notarization, DMG, PKG, MSI, or archive logic exists anywhere in the repo | `grep -ri 'codesign\|notariz\|dmg\|hdiutil\|\.pkg\|msi'` over `scripts/` and `.github/` returns zero hits |
| The release ships **bare binaries**; the macOS `.app` is assembled later, on the user's machine, by the installer | `scripts/release.sh` copies `target/release/{vak,vak-desktop,vak-delivery-worker}`; `crates/vak/src/install/bundle.rs` writes `Info.plist` at install time |
| You cannot notarize an artifact that does not exist until install time | consequence of the row above; this is the single structural blocker |
| Node/npm is required to build both frontends, and `npx` is required at runtime by every curated integration | `scripts/build.sh`, `scripts/release.sh`, `INTEGRATION_CATALOG` in `crates/vak-server/src/lib.rs` |
| Nothing detects or reports a missing Node to the user | no `node`/`npx` probe in `crates/vak/src/doctor.rs` or `crates/vak-core/src/health.rs` |

### Release

| Fact | Evidence |
|---|---|
| No release workflow exists | `.github/workflows/` contains only `ci.yml` and `chaos.yml` |
| A release describes **only the machine that ran it** — one platform key from `uname` | `scripts/release.sh`, `PLATFORM`/`ARCH`/`KEY` derivation |
| The update feed is trusted because it arrived over TLS; artifact digests come from the same document being trusted | `crates/vak/src/install/feed.rs`, `run_update` in `crates/vak/src/install/mod.rs` |
| No SBOM, no `cargo audit`, no `cargo deny`, no integrity manifest over the release set | absent from `ci.yml` and `scripts/release.sh` |
| `dist/` holds 60 version directories and 2.9 GB, untracked, never pruned | `ls dist`, `du -sh dist` |

### Install

| Fact | Evidence |
|---|---|
| `self install` is **not inert** — it seeds capabilities, creates `~/vak-home`, syncs services, starts the gateway, installs a desktop tray unit, and stops Telegram | `run_install` → `seed::seed_shared_capabilities` + `bootstrap_default_workspace_if_fresh` in `crates/vak/src/install/mod.rs` |
| Initial install seeding is tied to the platform-default install bootstrap; later explicit setup/update reconciliation targets the canonical Shared workspace | install bootstrap guard plus `seed_shared_capabilities()` callers in setup/update/services-sync |
| Seeds were install-only, so new standard capability content shipped in a release reached nobody who updated | `seed_shared_capabilities()` is invoked by setup/update paths and uses a manifest to advance untouched shipped skills/plugins while preserving edits |
| `uninstall --purge` removes the prefix, service units, and `data_home` — but leaves `~/vak-home`, which holds the entire Shared config layer, `.env` secrets, seeded skills, and installed plugins | `run_uninstall`; `default_workspace()` in `crates/vak-config/src/paths.rs` resolves outside `data_home()` |
| Uninstall does not remove the `/usr/local/bin/vak` symlink the installer itself told the user to create | `report_next_steps` suggests it; nothing reverses it |
| A legacy `com.vak.telegram` unit is still in the service table, kept alive by suppression logic in two separate places | `SERVICES` in `crates/vak-ops/src/services.rs`; `legacy_telegram_superseded` in both `run_status` and `run_services_sync` |
| Three concurrent ways to set a bot token, one explicitly labelled "kept for compatibility" | `/config/telegram-token`, `/config/bot-token/{surface}`, `/gateway/bots/{id}/token` in `crates/vak-server/src/lib.rs` |

### Setup / first run

| Fact | Evidence |
|---|---|
| `vak setup` does not exist | `Command` enum in `crates/vak/src/cli.rs` has no such variant |
| `docs/design/29-personal-os.md` claims a CLI wizard is complete. It is not | doc 36 recorded this in August; still true |
| No onboarding API exists | ~90 routes in `crates/vak-server/src/lib.rs`; none under `/onboarding` |
| Desktop first run is a folder picker plus a **dismissible** provider card | `crates/vak-desktop/ui/src/components/ProjectGate.tsx`, `SetupCard.tsx` |
| Picking a folder is also the act of trusting it | `ProjectGate.pick()` → `start_backend` |
| The setup card takes a provider and a key. It never discovers a model, never freezes a route, never asks about permission posture, never mentions channels, sandboxes, services, or dependencies | `SetupCard.tsx`, 111 lines total |
| Channel/bot onboarding exists only in the admin console, reachable only by someone who already knows the gateway is running and where | `docs/design/34-channel-onboarding.md`, `crates/vak-admin-ui/src/App.tsx` |

### The integration catalog

The code is already uniform: `INTEGRATION_CATALOG` in `crates/vak-server/src/lib.rs`
holds four entries — Tavily, Exa, Context7, Firecrawl — behind one shape,
one status projection, and one scoped read/write path. **Nothing in the
runtime privileges Tavily.** Its primacy is entirely documentation and
naming: a design doc titled "First-class Tavily integration", first position
in the array, and test fixtures that use `tavily` as the stand-in name for
"an MCP server". That is a product statement we are not making, so it goes.

### The honest summary

The runtime is finished and the front door is missing. Every mechanism the
lifecycle needs — transactional install, per-component digests, semver
update decisions, rollback, service generation, health projection — already
exists and is *idle*, because nothing gets far enough to use it. There is no
first install that is not a compile.

---

## Part II — Decisions

Taken 2026-09-02; these close doc 36's and 37's open questions.

| # | Decision | Consequence |
|---|---|---|
| D1 | **Per-platform native artifact, plus one bootstrap script.** macOS: `.dmg`. Linux: `.tar.gz` primary, `.deb`/`.rpm` secondary. Windows: whatever Windows supports (`.msi`), when the platform work exists. One `install.sh` covers macOS and Linux. | No single universal binary is pretended at. One URL, per-platform payload behind it. |
| D2 | **Ship unsigned now; sign and notarize in a later phase.** | The DMG is real but Gatekeeper shows a warning. Onboarding **owns that dialog explicitly** — the DMG window and the docs both state the exact right-click → Open sequence. We do not hide it, and we do not pretend the artifact is trusted. |
| D3 | **`--purge` is a full clean, `~/vak-home` included.** | Uninstall leaves zero residue; the next install is a genuine first run. Implemented as an explicit **preserve-allowlist**, not a delete-list, with a second confirmation that names what is about to die and refuses when the data root is a symlink. |
| D4 | **No integration is seeded.** All four are offered identically in the wizard, keyless and keyed alike. | Nothing reaches the network on our initiative. "Out of the box" means *the agent works*, not *every connector is live*. |
| D5 | **No integration is first-class.** Catalog order is alphabetical; doc 35 is deleted; `tavily` disappears as a test fixture name. | The catalog is a list of four peers. Adding a fifth is a data edit. |
| D6 | **Install places bits. Setup activates.** `self install` stops seeding, stops creating `~/vak-home`, stops syncing services, stops starting the gateway. | A fresh install starts no process. The wizard — desktop or CLI — is the only thing that ever activates anything, and it does so with the user watching. |
| D7 | **The web admin console is the onboarding surface.** It is present in *every* install — headless box, Linux server, macOS desktop — so it is the only surface that can carry one first-run experience everywhere. The desktop app points its webview at the same wizard; there is one implementation, not a desktop one and a web one. | One state machine, one rendering, three ways in. `vak setup` is terminal parity for people who never open a browser. |
| D9 | **A local setup server, not the durable gateway.** First launch starts a loopback-only, token-authenticated server that exists to serve the wizard, and opens the browser at it. It is not registered with launchd/systemd and dies when setup closes. | D6 survives: install still starts no durable service, and nothing unattended exists until Step 8. |
| D10 | **The wizard is written for someone who has never heard of an LLM harness.** Plain language, one decision per screen, no unexplained jargon, progressive disclosure for everything technical. | Explicit design contract in Part IV. A screen that needs the reader to already know what a "route" or "sandbox backend" is has failed. |
| D8 | **No legacy, no migration.** Every compatibility path listed in Part I is deleted outright, not deprecated. | Listed explicitly in Part VI so nothing is removed by accident. |

**Amendment to D7 (2026-09-25, docs/design/75-visual-refresh.md §6.2).** The
workspace client completes the provider and route steps in place, in the
"Connect an AI service" sheet (`crates/vak-client-ui/src/components/ConnectSheet.tsx`),
because leaving the app to connect a model was the first thing a new person
met. The sheet is not a second setup: it makes the wizard's own calls with
the wizard's scopes, so identical choices give identical configuration, and
`GET /onboarding` stays the one judge of ready. Every other step still opens
the wizard.

---

## Part III — The unified model

Three verbs. Each has one owner and one contract.

```
  ┌──────────┐        ┌──────────┐        ┌──────────┐
  │ INSTALL  │───────▶│  SETUP   │───────▶│   RUN    │
  └──────────┘        └──────────┘        └──────────┘
  places + verifies    chooses + activates  fails closed
  writes the manifest  writes config/secrets never infers consent
  starts nothing       starts services       never prompts headless
```

**Install** is `vak self install`, reached three ways — a DMG that drags an
`.app` into `/Applications` and runs the installer inside it on first
launch, a package whose postinst execs it, or the bootstrap script. All
three converge on the same one writer (doc 32 invariant 11, doc 37
invariant 14). It places components, verifies digests, writes the manifest,
and stops.

**Setup** is one resumable state machine with **one rendering** — the web
wizard served by the admin console (D7) — reached from the desktop app's
webview, a browser on any machine, or an SSH tunnel, plus a terminal parity
mode for people who want no browser. It owns every choice and every
activation: workspace, trust, provider, route, permission posture,
dependencies, capability seeds, integrations, channels, services, and the
first result. It also owns *which configuration layer each choice lands in*
(Part VI).

**Run** is unchanged and stays fail-closed. A non-interactive `vak exec`
against an unfinished setup refuses; it never infers a decision the wizard
would have asked for.

### The readiness projection

`OnboardingState` lives in `vak-core`, derived on every read from the same
authorities the rest of the system uses — never cached, never trusted from
a file. Deleting a key, moving a workspace, or revoking a permission makes
setup incomplete again on the next read, with no state to invalidate.

```rust
pub struct OnboardingState {
    pub install:      StepState,  // manifest verifies, components present
    pub dependencies: StepState,  // node/npx, git, optional docker — probed
    pub workspace:    StepState,  // chosen, readable, sessions home writable
    pub trust:        StepState,  // explicit decision on record, or safe-open
    pub provider:     StepState,  // credential present AND authenticated
    pub route:        StepState,  // provider+model frozen as one atomic write
    pub permission:   StepState,  // posture chosen and persisted
    pub sandbox:      StepState,  // backend identified, or fail-closed reason
    pub capabilities: StepState,  // seeds materialized at the current version
    pub integrations: StepState,  // informational; never blocks core_ready
    pub channels:     StepState,  // informational; required for unattended
    pub services:     StepState,  // only meaningful after an always-on choice
    pub first_result: StepState,
    pub core_ready:       bool,
    pub unattended_ready: bool,
}
```

`core_ready` = install + workspace + trust + provider + route + permission +
writable sessions home. `unattended_ready` additionally = services healthy +
at least one channel bound + an approver surface configured + no silent
auto-deny path on essential work.

Every `StepState` failure carries four fields, always: **what failed**,
**what is still safe**, **one primary repair action**, and the original
typed error behind a disclosure. "Setup failed" as a single message is
forbidden — provider auth, model discovery, sandbox availability, and
backend boot are four different failures with four different repairs.

Presentation-only metadata (last step, dismissals, usage mode, local
milestone timings) lives in `<data_home>/onboarding.json` and can alter
**presentation only**. A corrupted or forged file can never grant a
capability.

---

## Part IV — The first run

One state machine, rendered once, in the web admin console (D7). The desktop
app opens that same page in its webview; a headless server reaches it over
loopback or an SSH tunnel; `vak setup` is terminal parity for anyone who
wants no browser at all. Identical choices produce byte-identical config,
because there is one implementation of the choice.

### How the wizard is reached

| Install | First run |
|---|---|
| macOS DMG | launch `Vakyartha.app` → it starts the local setup server → the wizard opens in the app's own window |
| Linux tarball / package | `vak setup` prints one URL and opens the browser where there is one; `--no-browser` prints and waits |
| Headless / remote | `vak setup --print-url`, reach it over an SSH tunnel; the token is printed once, to the terminal only |
| Anyone, any time | `vak setup --terminal` runs the whole thing as prompts |

The setup server binds loopback only, requires the printed bearer token, and
is not a durable service (D9). It shuts down when setup completes or the
window closes. Nothing unattended exists until Step 8 says so.

### Design contract (D10)

This wizard is the first thing a non-technical person sees, and most of what
it asks about is genuinely technical. The rules that resolve that tension:

- **One decision per screen.** Never a settings page wearing a wizard's hat.
- **Plain words in the question, precise words in the disclosure.** The
  screen asks "How much should vak be allowed to do on its own?"; the
  "What does this mean exactly?" expander says `workspace-write`, names the
  sandbox backend, and links the design doc.
- **Never show an ID where a name will do.** Model IDs, chat IDs, bot IDs,
  and unit names live behind disclosure, not in the primary line.
- **Every screen states what happens next**, so no click is a leap.
- **Progress is honest.** A step that is optional says so; a step that was
  skipped stays visibly skippable, not silently gone.
- **Recommended is marked, and unrecommended is not hidden.** Full access is
  reachable, clearly separated, and never the default.
- **Every failure is a sentence a person can act on**, with the typed error
  underneath for the person who wants it.
- **Nothing spins without saying what it is waiting for.** "Asking OpenAI
  which models your key can reach" beats a spinner.
- **The wizard is beautiful on a phone too** — an operator approving their
  first Telegram chat is often holding the phone that sent the message.

### Step 0 — Welcome and mode

What do you want vak to be?

- **Desktop app** (recommended) — no durable services.
- **Command line** — shows the installed path, offers the PATH link, continues.
- **Always-on assistant** — adds the channels and services steps later.

This selects which later steps appear. It never affects permissions.

### Step 1 — Dependencies

Probe and report, do not silently proceed:

| Dependency | Needed for | Missing means |
|---|---|---|
| Node / `npx` | all four curated integrations | integrations step is shown disabled, with the exact install command for this platform |
| `git` | worktrees, PR flow, diff review | those features are marked unavailable, not broken at first use |
| Docker | optional container sandbox backend | the sandbox step reports the OS backend it will use instead |

A missing optional dependency never blocks `core_ready`. It is stated once,
here, instead of surfacing as a confusing failure three screens later.

### Step 2 — Workspace and trust

Pick a directory. Default offered: `~/vak-home`, created on demand — the
same canonical path durable services resolve independently (invariant 18).

Scan for onboarding-relevant facts only: git status, `.vak/config.toml`,
the project secret scope, and which privileged sections the project requests
(hooks, MCP servers, provider endpoint, sandbox, gateway, permission grants,
allow rules, prompt layers).

If none exist, open it — no prompt. If any exist, a dedicated trust review
lists exactly what the project asks for, and offers **Open safely** (project
privileged config and secret scope stay demoted), **Trust this workspace**
(canonical-path decision persisted), or **Cancel**.

Selecting a folder is no longer the act of trusting it. This is the single
biggest behavioral change to the desktop surface.

### Step 3 — Provider and route

Transactional from the user's point of view:

1. Choose a provider from the live inventory. Configured ones are marked;
   Ollama shows as local/keyless when reachable.
2. Enter the credential if one is required. Stored only through
   `Core::set_provider_key` into the canonical Shared secret scope — never
   argv, never onboarding state, never logs.
3. **Authenticate by discovering models.** A stored key is not success.
4. Choose a discovered model. An exact-ID escape hatch exists for providers
   that cannot enumerate, and is labelled *route unverified*.
5. Provider and model save atomically as one route (invariant 17, 9).

A key that stores and then fails discovery leaves the route **unchanged**
and says so precisely.

### Step 4 — Safety posture

Three plain-language presets over the existing modes — no fourth mode:

- **Inspect only** → `read-only`
- **Work with approval** → `workspace-write` (recommended)
- **Unrestricted** → `full-access`, visually separated, separate explicit
  confirmation, never recommended, never auto-selected (invariant 13).

The effective sandbox backend is shown beside the choice. A restricted mode
with no available containment backend fails closed here with the exact
remedy, rather than at first tool call.

### Step 5 — Capabilities

Show what the seed brings: the six Shared skills and two starter plugins in
`crates/vak-core/src/seed.rs`, as a reviewable list with a per-item
toggle. Materialize on confirm, into the chosen workspace's Shared layer —
**not** as a side effect of install, and **not** conditional on the prefix.

Seeds are versioned. A release that changes a seed offers the delta on next
setup or update; it never silently overwrites a file the user edited.

### Step 6 — Integrations

All four catalog entries, rendered identically: name, one-line description,
what it needs, documentation link, key field where required, enable toggle.
Alphabetical. None enabled by default (D4). Greyed with an explanation when
Node is absent. Enabling writes the managed server definition and hot-applies
it; the key goes to the canonical secret scope and is never returned to any UI.

### Step 7 — Channels (always-on mode only)

Add a bot: choose a surface (Telegram / Discord / Slack), paste a token,
name it. The wizard creates the `Bot` identity, provisions its per-bot
service unit, and — critically — walks the operator through **one approval**:
send a message to the bot now, watch it land as a `pending` entry, approve
it, and see the reply. The allowlist stays fail-closed throughout
(invariant 15); the wizard teaches the loop instead of leaving it to be
discovered after a silent 403.

Zero bots is a valid, complete outcome.

### Step 8 — Activate

Only now do services get written and started, and only for what the mode
requires: gateway for always-on, desktop tray unit if the desktop was
chosen and a `vak-desktop` binary shipped, one unit per configured bot.
Each unit captures the workspace identity chosen in Step 2 (invariant 18).

The activation summary states, per service, what was written, where, and
whether it came up — with the log path when it did not.

### Step 9 — First result

Offer one read-only starter task:

> Look around this workspace and tell me, briefly and in plain words, what
> is here and what you could help me with — and what you would need from me
> to do more. Do not change anything.

(`vak_core::onboarding::FIRST_TASK_PROMPT`, the one copy both the web wizard
and `vak setup` use. The default workspace is a home for any kind of work, so
the starter task is general-purpose rather than a code walkthrough.)

The run is capped to read-only **regardless of the posture chosen in
Step 4**. On completion, a summary shows files and tools observed,
permission decisions taken, the provider and model, token/cost data where
available, and a prominent **Open receipt** — because the receipt is the
product, and it should be seen in the first five minutes rather than found
behind an icon in week two.

Skipping loses nothing.

### Re-entry, reinstall, and repair

Setup is idempotent and derived, so all four of these are the same code path:

- **Interrupted** — resumes at the first incomplete step.
- **Reinstall over an existing install** — every step reports *already
  satisfied*; the wizard becomes a review screen with the seed delta, if
  any, and an explicit "change anything" affordance. It never re-asks a
  settled question and never re-writes a settled answer.
- **Broken after an update** — the same screen shows exactly which steps
  regressed and why.
- **`vak doctor`** — renders the identical projection in report form, and
  `--repair` acts only on steps with a known mechanical fix (invariant 19).

---

## Part V — Bundle and release

### Artifacts per release

| Platform | Primary | Also | Notes |
|---|---|---|---|
| macOS (aarch64, x86_64) | `Vakyartha-<version>-<arch>.dmg` containing `Vakyartha.app` | `install.sh` | Bundle built **in the release**, not at install time. Unsigned for now (D2). |
| Linux (x86_64, aarch64) | `vak-<version>-<arch>.tar.gz` | `install.sh`, later `.deb`/`.rpm` | Package payload unpacks to staging; postinst execs `vak self install`. No unit files in the package — units stay generated. |
| Windows | — | — | Blocked on platform work (Part VII). Say "macOS and Linux" until then. |

Plus, per release: `SHA256SUMS`, a signed integrity manifest over the whole
set, an SBOM, and `release.json` merged from every matrix leg.

### Building the bundle once

`crates/vak/src/install/bundle.rs` grows a "materialize a Vak bundle at this
path" entry point. The release workflow and `self install` both call it, so
there stays exactly one definition of what a Vak bundle *is*, while the
artifact that ships is one a notarization service could actually accept
later.

### Release ownership

`scripts/release.sh` gains `--platform-key`, so a CI leg declares what it
built instead of inferring it from `uname`. A new
`.github/workflows/release.yml` runs a matrix — macos-14 (aarch64),
macos-13 (x86_64), ubuntu (x86_64, aarch64) — each leg running the existing
gates and uploading its component set, with a final leg merging every
platform key into one feed. Laptop releases stop being the mechanism.

### Evidence

Adopted from doc 37 invariants 16–18, unchanged in intent:

- **Integrity manifest** over the release *set* — name, exact byte length,
  SHA-256 per artifact, plus a digest over the manifest's own canonical
  payload. The verifier rejects a missing, extra, duplicated, or
  path-escaping entry, and any size or digest drift. Per-file checksums
  prove a file was not corrupted; only this proves the set was not edited.
- **Signed feed.** `release.json` carries a detached signature over its
  canonical bytes; the public key compiles into the binary; an unsigned or
  wrongly-signed feed is refused *before any digest is consulted*. Today
  `self update` trusts whatever document the URL returns.
- **SBOM + advisory gate.** CycloneDX from `Cargo.lock` and both frontend
  lockfiles; `cargo audit` and `cargo deny` in CI and in the release gate.
  **An unavailable check is a failure, not a pass** — a tool that is not
  installed, a service that times out, fails the release. Silence is never
  evidence.

### Housekeeping

`dist/` is release output, untracked, and unbounded — 60 directories, 2.9 GB.
`scripts/release.sh` prunes to the last N (default 3) after a successful
run, and `scripts/vak.sh` grows a `clean-dist` verb. Once CI owns releases,
local `dist/` is scratch and should look like it.

---

## Part VI — Configuration, secrets, and inheritance

Setup writes into a layered configuration system that already exists and is
deep. It must write into that system correctly, not beside it. This part is
the contract for *where every choice lands*.

### The layers, as built

```
  seed (compiled in)                       prompts only — never a file
    └─ Shared / user layer                 ~/vak-home/.vak/config.toml
       │                                   ~/vak-home/.env       (secrets)
       │                                   ~/vak-home/.vak/skills, plugins, hooks
       └─ project layer                    <workspace>/.vak/config.toml
          │                                <workspace>/.env      (secrets)
          └─ surface                       telegram | discord | slack
             └─ bot                        one identity, own token/route/policy
                └─ chat                    one allowlist entry
                   └─ agent role / session pins, frozen at admission
```

Resolution rules already in force (AGENTS.md invariants 17, 20, 23, 27, 28):

- **Narrowest wins** for single-valued things: `route`, `permission_mode`
  (capped, never escalating), `identity`, `operating_rules`.
- **Concatenate and de-duplicate** for things that can only narrow:
  `guardrails`, `surface_note`, `ChannelPolicy` deny lists.
- **File presence is the inheritance switch.** A project layer stores
  *intent*, never a copied snapshot of effective user values.
- **A GET that seeds a same-shape PUT reports only the layer that PUT
  writes** — never the merged view (invariant 21). Violating this silently
  copies an inherited global value into the project file on the next save.
- **Secrets never live in TOML.** Lookup is process override → real
  environment → `.env`. **The canonical user `.env` is
  `~/vak-home/.env`** — `user_env_path()` in `crates/vak-config/src/lib.rs`
  resolves it under `default_workspace()`, not under `data_home()`. Shared
  config sits beside it at `~/vak-home/.vak/config.toml` (`global_path()`).
  `README.md` still documents the pre-1.0.3 `data_home()/.env` location and
  is wrong; correcting it is part of Tier 3.

  This is also why D3 matters more than it first reads: `~/vak-home` is not
  "a workspace the user happens to have" — it *is* the configuration and
  secret home. A purge that spared it would spare everything that makes a
  reinstall not a first run.

### What setup writes, and where

Every wizard step declares its target layer explicitly. This table is the
contract; a step that cannot name its layer is not ready to ship.

| Step | Writes | Layer | Notes |
|---|---|---|---|
| Workspace | gateway workspace selection | `data_home()/gateway/default-workspace` | not config; a service-identity fact |
| Trust | canonical-path trust decision | trust store | never a project file — a project cannot trust itself |
| Provider key | `<PROVIDER>_API_KEY` | canonical user `.env` | via `Core::set_provider_key` only |
| Route | `provider` + `model`, atomically | **Shared** by default; project when the user says "just this workspace" | one atomic write, never two |
| Permission posture | `permission_mode` | Shared, unless scoped | revoke-before-apply |
| Seeds | skills, plugins, hooks | Shared (`~/vak-home/.vak/`) | skills/plugins are manifest-versioned: untouched shipped content advances, edited content is preserved; hooks/network defaults seed only when empty |
| Integrations | MCP server definition | Shared by default, project on request | key to the corresponding `.env` |
| Channels | `Bot` identity, policy, route | gateway state + per-bot `.env` token var | token var name is dynamic per bot id |
| Services | unit files | platform service manager | captures the Step 2 workspace identity |

**The default layer is Shared.** A new user setting up their machine is
configuring *themselves*, not one folder. Every step that can write Shared
offers a visible "only this workspace" alternative, and shows which layer
the current value came from.

### Provenance is shown, always

Every setting the wizard or Settings displays carries its origin: **set
here**, **inherited from Shared**, or **default**. An inherited value is
rendered differently from a local one, and changing an inherited value asks
which layer to write. This is already true of some admin surfaces and not
others; setup makes it uniform, because the single most confusing thing
about a deep inheritance chain is a value you cannot attribute.

### Secrets: one path in, no path out

- Written only through the canonical secret API, into the canonical `.env`,
  owner-only, atomically replaced.
- Never returned by any API once set — not to the desktop, not to the admin
  console, not in a config dump. A set key reads back as *present*.
- Never in argv, onboarding state, logs, receipts, session ledgers, or
  service unit files. A unit file carries the env var *name*; the value is
  read at process start.
- Never ambient in a Bash or MCP subprocess (invariant 12) — injection stays
  explicit and recipient-scoped.
- Revocation invalidates the cached provider client and the discovered-model
  cache in the same operation.

### Config in the lifecycle

| Event | Config behavior |
|---|---|
| **Install** | touches no config, no `.env`, no workspace. The one exception today — pinning `VAK_GATEWAY_TOKEN` — moves to setup's activation step, where the console it authenticates is actually being started. |
| **Update** | preserves every user-edited layer and applies standard skill/plugin seed deltas only when the installed content still matches Vak's recorded shipped digest; new content is added, edited or independently installed content is left untouched. Config schema is additive-only within a major version. |
| **Reinstall** | reads existing config and skips every settled step. It never rewrites a settled answer. |
| **Uninstall** | removes units and the prefix; config and secrets survive. |
| **Uninstall --purge** | removes `data_home()` **and** `~/vak-home` (D3) — which means the Shared config layer, the canonical `.env`, every seeded skill, every installed plugin, and every session. Preserve-allowlist, symlink refusal, second confirmation naming exactly what dies. Project-layer `.vak/` directories inside other repositories are **not** touched: they are the user's files in the user's projects, and a purge is not a licence to walk the filesystem. That exception is stated in the confirmation. |

### Config doctoring

`vak doctor` grows a config section that answers the questions a deep
inheritance chain makes hard: which layers exist, which are trusted, what
the effective route/permission/sandbox is and which layer set it, which
`.env` files were loaded, which keys are present (never their values), and
which project settings are demoted because the workspace is untrusted.

---

## Part VII — The cut, and the contract after it

Everything above describes one disruptive release. This part is the more
important half: **what guarantees we make from that release onward, and what
mechanism enforces them**, so this stabilization is done once and never
relitigated.

### VII.1 — 2.0.0 is the base. Nothing before it is supported.

This release is the **baseline**, not a milestone on a continuum. Every
version before it is stale, unsupported, and not a thing any code, doc, or
person is obliged to accommodate. That is a declaration with teeth in six
places:

**1. The binary refuses pre-base state, clearly.** A data home, manifest,
gateway store, or config written before 2.0.0 is not half-read and not
migrated. It is refused by one shared message that names the file, says the
install predates the supported baseline, and gives the one command that
resolves it:

```
This data was written by vak 1.0.3, which predates the 2.0.0 baseline.
Versions before 2.0.0 are not supported and cannot be upgraded in place.

  vak self uninstall --purge     then install 2.0.0 and run setup

Your project files are untouched; only vak's own state is removed.
```

**2. The update feed carries no pre-base version.** `release.json` starts at
2.0.0. `self update` from a 1.x install does not resolve — it reports the
baseline and points at the same command. We do not ship an upgrade path we
have not tested, and we will not test one to a line we do not support.

**3. `AGENTS.md` states it as a contract.** A new invariant: *2.0.0 is the
supported baseline; no code may accept, migrate, or special-case state
written before it. A compatibility branch for a pre-baseline shape is a
review failure, not a kindness.* This is what stops the tolerances growing
back one sympathetic PR at a time.

**4. `CHANGELOG.md` restarts.** Entries before 2.0.0 are removed, and the
file opens by saying the project's supported history begins here. Nothing is
actually lost — git holds every prior entry — and what the file gains is
that it no longer reads as a supported lifecycle stretching back to 0.2.1.

**5. Every design doc is brought to the baseline or deleted** (Part IX).
A doc describing behavior that only existed before 2.0.0 is not history, it
is a trap for whoever reads it next.

**6. `README.md` describes 2.0.0 only.** No "since 0.11", no version
matrices, no upgrade notes. One supported version, described in the present
tense.

**The cut happens exactly once.** Everything in Part VIII Tier 2 lands in
2.0.0; the day it tags, Tier 2 closes. Anything discovered afterwards is
additive or it waits for 3.0.0 — and 3.0.0 is a rare, announced event, not
something we back into. A second "we need one more break" release is the
specific failure this entire part exists to prevent.

### VII.2 — The durable state registry

You cannot promise to preserve what you have not enumerated, and prose
enumerations rot. The registry is **code**, in `vak-core`, and every durable
artifact vak owns is declared in it:

```rust
pub struct StateEntry {
    pub path:      StatePath,   // relative to data_home | shared | project
    pub owner:     &'static str,// crate that writes it
    pub schema:    Option<u32>, // None = schemaless (JSONL append log)
    pub kind:      StateKind,   // Ledger | Config | Secret | Derived | Cache
    pub on_update: Survives,    // Untouched | AdditiveOnly | Rebuilt
    pub on_purge:  Purge,       // Remove | Preserve
}
```

Declared from what actually exists today (verified on disk, 2026-09-02):

| Location | Contents | Kind | On update | On purge |
|---|---|---|---|---|
| `data_home()/sessions/` | append-only session JSONL | Ledger | untouched | remove |
| `data_home()/gateway/` | `allowlist.json`, `bindings.json`, `bots.json`, `default-workspace` | Config | additive-only | remove |
| `data_home()/operations/` | `incidents.jsonl`, `actions.jsonl` | Ledger | untouched | remove |
| `data_home()/agent-network/` | inter-agent state | Config | additive-only | remove |
| `data_home()/security-events.jsonl` | audit log | Ledger | untouched | remove |
| `data_home()/memory/`, `checkpoints/`, `skill-proposals/`, `trusted/` | user state | Ledger/Config | untouched | remove |
| `data_home()/cost-log.jsonl`, `routing-evidence.jsonl`, `tasks.json`, `desktop.json` | ledgers + task config | Ledger/Config | additive-only | remove |
| `data_home()/locks/` | runtime locks | Derived | rebuilt | remove |
| `cache_home()` | FTS index, WAL sidecars | Cache | rebuilt | remove |
| `~/vak-home/.vak/config.toml` | **Shared config layer** | Config | additive-only | remove |
| `~/vak-home/.env` | **canonical secrets** | Secret | untouched | remove |
| `~/vak-home/.vak/skills/`, `plugins/` | seeded + installed capabilities | Config | seed-delta only | remove |
| `<prefix>/…/install.json` | install manifest | Config | rewritten by design | remove |
| `<workspace>/.vak/`, `<workspace>/.env` | project layer in other repos | Config/Secret | untouched | **preserve** |

Two enforcement tests, both cheap and both load-bearing:

1. **No unregistered writes.** A test drives a full setup + a real turn
   against a temporary home, then asserts every file that appeared is
   covered by a registry entry. A new durable file that nobody declared
   fails the build — which is exactly how this list stops rotting.
2. **Purge matches the registry.** `--purge` removes precisely the `Remove`
   entries and nothing marked `Preserve`, asserted on the filesystem.

The registry also becomes the single source for `--purge`'s
preserve-allowlist (D3), for backup coverage, and for the upgrade gate
below — three things that currently each carry their own half-list.
`crates/vak-core/src/backup.rs` today hardcodes 5 directories and 4 files;
it reads the registry instead.

### VII.3 — The compatibility contract, from 2.0.0 onward

Five rules. They are deliberately strict, because the cost of strictness is
a little awkwardness at write time and the cost of laxity is another 2.0.0.

1. **Additive-only within a major version.** A field may be added. A field
   may not be removed, renamed, retyped, or have its meaning changed. If
   you need that, you need a major version — and majors are rare and
   announced, not discovered.
2. **Readers ignore unknown fields.** A file written by a newer minor must
   load in an older one. `#[serde(default)]` on every added field, no
   `deny_unknown_fields` on any durable type.
3. **Writers round-trip what they did not understand.** A reader that
   parses, mutates, and rewrites a file must preserve fields it does not
   know about. Otherwise an older binary silently strips a newer one's
   data — the quietest possible data loss, and the one nobody notices until
   it matters.
4. **A newer schema is refused loudly, never half-read.** The pattern in
   `crates/vak-plugin/src/lib.rs` (`schema > REGISTRY_SCHEMA` ⇒ typed
   error) becomes universal: every schemaed file, one shared helper, one
   message that names the file, both versions, and what to do.
5. **Ledgers are append-only forever.** Session JSONL, security events,
   operations ledgers, cost log. New information is a new entry type, never
   a changed one (invariants 1 and 2). This is the rule that makes "no
   migration" sustainable rather than merely stated.

**Migrations.** Within a major: none exist, because none can be needed. At
a major: exactly one, one-way, tested against a real fixture from the
previous major, and it takes a full backup first. A migration that cannot
be tested against a real old-version fixture does not ship.

### VII.4 — Update is non-destructive, and proves it

`self update` today is transactional over **binaries** and touches state not
at all — which is correct but unenforced, and one careless future commit
away from being untrue. Three additions:

- **Snapshot before mutate.** Any update that will touch a registry entry
  takes a timestamped snapshot first, through the existing backup machinery
  (`vak_core::backup`, now registry-driven). Kept for N updates, pruned
  after. `self rollback` restores binaries *and* that snapshot together.
- **State joins the transaction.** The existing `Transaction` in
  `crates/vak/src/install/transaction.rs` gains state entries, so a failed
  update rolls both back or neither.
- **Seeds evolve without clobbering.** Each seeded file records the digest
  of what we shipped. On update: unchanged from our digest ⇒ silently
  updated to the new seed; edited by the user ⇒ **never** touched, and the
  delta is offered in the wizard's review screen. A user's edited skill is
  their file, permanently.

### VII.5 — The upgrade gate (this is the anti-loop mechanism)

Everything above is a promise. This is the machine that keeps it.

On every release, and on every PR that touches a registry entry or an
installer path, CI runs:

```
1. install the PREVIOUS released artifact (from the real feed)
2. drive setup to a fully-configured fixture, non-interactively:
   workspace + trust, provider (mock), route, posture, seeds,
   2 integrations, 2 bots, 1 approved chat, 1 completed session,
   1 scheduled task, 1 outbox record, edited seed skill
3. snapshot every registry entry (digest per file)
4. `self update` to HEAD
5. assert, per registry entry:
     Untouched      → byte-identical
     AdditiveOnly   → loads, every prior field preserved, no field dropped
     Rebuilt        → regenerates, and regenerates equivalently
     Preserve       → still present
6. assert setup reports fully configured and re-runs NOTHING
7. assert the edited seed skill is untouched and the unedited one advanced
```

Then the reverse leg, which is the one people forget: **the older binary
must still read what the newer one wrote** (rule 2), so step 4 is repeated
downgrading, and the old binary must load every file without error.

**An unavailable check is a failure, not a pass** (doc 37 invariant 18). No
previous artifact to fetch, a fixture that will not build, a mock provider
that will not start — each fails the release. Silence is never evidence.

This gate is what makes "we do not do this again" a property of the build
rather than an intention.

### VII.6 — Setup runs once, and afterwards is a review

The user-facing half of not looping. After the first completed setup, every
subsequent install, reinstall, or update lands on a **review screen**, never
the wizard:

> Everything is already set up. — workspace, provider, model, posture,
> 4 skills, 2 integrations, 1 bot, gateway running.
> *[Two seeded skills have newer versions — review]*  *[Change something]*

Because readiness is derived, not remembered (Part III), this is automatic:
every step reports *already satisfied* and renders as a line, not a
question. A step that genuinely regressed — a revoked key, a moved
workspace — is the only thing that ever surfaces as a question again, and it
surfaces alone rather than dragging the user back through nine screens.

The corresponding rule for the terminal: `vak setup` on a configured machine
prints the same review and exits 0 without prompting.

### VII.7 — What each lifecycle event does to state, definitively

| Event | Binaries | Config | Secrets | Ledgers | Services | Setup |
|---|---|---|---|---|---|---|
| **First install** | placed, verified | none written | none | none | none registered | not run |
| **First setup** | — | Shared written | written once | created | registered on request | full wizard |
| **Update** | replaced transactionally | additive-only, snapshotted | untouched | untouched | units re-synced, identity preserved | review only |
| **Reinstall** | replaced | untouched | untouched | untouched | re-synced | review only |
| **Uninstall** | removed | preserved | preserved | preserved | removed | — |
| **Uninstall --purge** | removed | removed | removed | removed | removed | next run is a true first run |
| **Downgrade** | replaced | must still load | untouched | untouched | re-synced | review only |

Every cell in that table is asserted by a test named after it. A row that
cannot be tested is a row we are not entitled to promise.

---

## Part VIII — Legacy: the full sweep

Per D8: golden paths only. Every alternative way of doing something that
already has a canonical way is deleted, not deprecated — together with the
tests that pinned the old behavior, and the doc paragraphs that described it.

Sorted into three tiers because they carry very different risk.

### Tier 1 — Dead code paths (delete outright; no user cost)

| # | Legacy | Location | Golden path |
|---|---|---|---|
| 1 | `com.vak.telegram`, the bot-id-less unit, plus `legacy_telegram_superseded` suppression in **two** places | `SERVICES` in `crates/vak-ops/src/services.rs`; `run_status` and `run_services_sync` in `crates/vak/src/install/mod.rs` | per-bot units only |
| 2 | `PUT/DELETE /config/telegram-token` and `/config/bot-token/{surface}` | `crates/vak-server/src/lib.rs` | `/gateway/bots/{id}/token` only — a surface is a transport, not a credential slot (invariant 23) |
| 3 | Legacy single-slot delivery token: `resolve_surface_token`'s `legacy_env_var` branch and the arbitrary-bot fallback behind it | `crates/vak-server/src/delivery.rs` | every outbound reply resolves through the `(surface, bot_id)` adapter map; a target naming an unconfigured bot fails loudly |
| 4 | `.vak/SYSTEM.md` whole-prompt override | `crates/vak-core/src/lib.rs` | the layered prompt blocks (doc 45) are the only way to set identity and rules |
| 5 | Install-time activation: `seed_shared_capabilities` + `bootstrap_default_workspace_if_fresh` | `crates/vak/src/install/mod.rs` | both become setup actions (D6) |
| 6 | Legacy tray launch path | `crates/vak-tray/src/main.rs` | the `com.vak.desktop` unit is the only thing that puts a tray on screen |
| 7 | `feeds.toml` caller-compatibility branch | `crates/vak-server/src/feeds.rs` | one feed-config shape |
| 8 | Dismissible `SetupCard` and trust-implying `ProjectGate` | `crates/vak-desktop/ui/src/components/` | the wizard (D7) |

### Tier 2 — Withdrawn: these are the contract, not legacy

**This tier was wrong, and executing it would have broken the contract
stated three parts earlier in this same document.** Recorded rather than
quietly deleted, because the mistake is instructive.

It listed `#[serde(default)]` tolerances — a `WorkReceipt` with no
provider, a session header with no route ladder, a `RouteLeg` with no
credential id, an empty circuit-breaker key — as "backward compatibility to
withdraw". They are not. VII.3 rule 2 *requires* exactly this mechanism:
every added field carries `#[serde(default)]` so a file written by a build
that predates the field still loads. Deleting them would mean a 2.1 reader
could not open a 2.0 file, which is the opposite of additive-only.

The remaining item, two-segment `surface:chat` allowlist keys, is not
legacy either: that is the shape an operator writes by hand in
`gateway.chat_allowlist`, documented in `docs/design/05-config.md` and
required by AGENTS.md invariant 24. `legacy_key_for` resolves a
config-written row against a bot-scoped inbound key, which is live
behaviour, not a migration.

What *was* wrong is the naming. Calling a permanent forward-compatibility
default "legacy" invites precisely the deletion attempted here, so the
comments and tests are renamed to say what they mean: a file written before
this field existed still loads, and always will.

The lesson generalises. **"Delete all legacy" is a real instruction with a
real exception**: a tolerance that lets an *older reader* open a *newer
file*, or a newer reader open an older one, is the compatibility contract
doing its job. Legacy is a second way of doing something that has a
canonical way (invariant 30) — not every branch that handles an absent
field.

### Tier 3 — Documentation and naming legacy

| # | Item | Action |
|---|---|---|
| 14 | `docs/design/35-tavily-integration.md` | delete (D5). Its still-true operational content — managed server definition, key never returned to the browser, hot-apply on enable/disable, no user-controlled `network` flag — moves into an integration-catalog section of `docs/design/09-extensibility.md`, written about all four peers |
| 15 | `tavily` as the stand-in fixture name | rename to a neutral name in `crates/vak-core/src/lib.rs`, `crates/vak-server/src/core_pool.rs`, `crates/vak-mcp/tests/mcp_roundtrip.rs`; catalog order becomes alphabetical |
| 16 | `docs/design/36-first-run-onboarding.md`, `docs/design/37-distribution.md` | delete; superseded here. Two stale plans a reader must reconcile is worse than one current one |
| 17 | `docs/design/29-personal-os.md`'s claim that the CLI wizard shipped | correct in the same commit that ships `vak setup` |
| 18 | `README.md` quick start leading with clone-and-compile | download, launch, done. Source builds move to Contributing |
| 19 | `dist/` — 60 stale version directories, 2.9 GB | prune to the last 3 after a successful release; add `scripts/vak.sh clean-dist` |

### The rule going forward

One canonical way per capability. A second way needs a design doc saying why
the first is insufficient, and it replaces the first rather than joining it.
Every "kept for compatibility" comment introduced from here is a review
failure.

---

## Part IX — Documentation and contract cleanup

Per the baseline declaration (VII.1 item 5): a doc that describes
pre-baseline behavior is not history, it is a trap. Everything in
`docs/design/` is either brought to 2.0.0 or deleted. Git keeps whatever is
removed, so nothing is actually lost — what is gained is that every file a
reader opens is true.

### Deleted

| Doc | Why |
|---|---|
| `docs/design/35-tavily-integration.md` | its premise — one integration is first-class — is withdrawn (D5) |
| `docs/design/36-first-run-onboarding.md` | superseded by this doc |
| `docs/design/37-distribution.md` | superseded by this doc |
| `docs/design/27-vakyartha-adoption.md` | 547 lines of provenance for mechanisms that already landed in code; a study of another project is not our contract |
| `docs/design/06-research-notes.md` | same shape: what we took from other harnesses. Interesting once, binding never |
| `docs/design/16-achievements.md` | states outright that it is a historical record of v0.1.x — pre-baseline by definition |
| `docs/design/40-competitive-landscape.md` | a market document living in a design directory, and the second file claiming number 40 |
| `docs/design/43-self-evolving-agent.md` | 2,248 lines marked *no behavior implemented*; a plan for something that does not exist |

### Rewritten to the baseline

| Doc | Change |
|---|---|
| `AGENTS.md` | add the baseline invariant (VII.1 item 3) and the one-canonical-way rule; update invariants 23 and 24, which currently describe the legacy token slot and two-segment allowlist keys being deleted in Part VIII |
| `CHANGELOG.md` | restart at 2.0.0; the file opens by stating that supported history begins here |
| `README.md` | quick start becomes download → launch → done; source builds move to Contributing; correct the stale `data_home()/.env` secret path to `~/vak-home/.env`; drop the version badge that can drift |
| `docs/design/00-roadmap.md` | restate against 2.0.0; phases that shipped stop being listed as plans |
| `docs/design/29-personal-os.md` | delete the false claim that the CLI wizard shipped |
| `docs/design/32-release-engineering.md` | amend for the CI release pipeline, bundle-built-in-release, feed signing, and pre-baseline refusal; it keeps ownership of the installed lifecycle |
| `docs/design/34-channel-onboarding.md` | remove the legacy key and token paths deleted in Part VIII; point channel onboarding at the wizard's Step 7 |
| `docs/design/05-config.md` | absorbs the former doc 44, which described the same layering from a second angle; doc 44 is deleted. Part VI cites it instead of restating it. It also carries the integration-catalog contract rescued from doc 35, written about four peers |
| `docs/design/40-harness-engineering.md` | heading now carries its number; content stands |

### Not renumbered, deliberately

`docs/design/` has gaps at 18, 19, and 21, and had a collision at 40 that
deleting the competitive-landscape doc resolved. Renumbering the set
contiguously would invalidate several hundred `docs/design/NN` citations in
code comments for no reader benefit, so the gaps stay. `scripts/check_doc_paths.py` already fails CI on a citation
that stops resolving, which is the property that actually matters.

---

## Part X — Phases

Each phase is independently shippable and leaves the tree working.

### S0 — Declare the baseline and clean the record

Cheap, fast, and it makes every later phase legible.

- Set the version line to `2.0.0`.
- Add the baseline invariant to `AGENTS.md` plus the one-canonical-way rule.
- Restart `CHANGELOG.md`; rewrite `README.md`'s quick start and correct the
  stale secret path.
- Execute Part IX: delete the seven superseded docs, rewrite the ten listed,
  resolve the number-40 collision.
- Add the shared pre-baseline refusal message and wire it to the manifest
  version check (the one place that can already tell).

**Exit:** every file in `docs/design/` describes 2.0.0 or does not exist.
`scripts/check_doc_paths.py` and `scripts/check-version.sh` pass. A 1.x
manifest produces the refusal message, not a stack trace.

### S1 — Split install from setup, and project readiness

- Strip activation out of `run_install` (D6): no seeds, no workspace
  creation, no service sync, no gateway start, no token pinning.
- `OnboardingState` in `vak-core`, derived on every read from real
  authorities; `GET /onboarding`; `vak setup status --json`.
- Typed failure categories with the four-field error shape.
- Config provenance in the projection: every step reports which layer set
  its value (Part VI).
- Delete Tier 1 legacy items 1, 2, 3, 5, 6, and 7.

**Exit:** a fresh install starts no process, writes no workspace, seeds
nothing. `vak setup status --json` on a clean data home reports each step
accurately — note that a keyless local provider (Ollama) or an ambient
environment credential legitimately makes the provider step *satisfied* on
a clean home, so the criterion is accuracy, not uniform emptiness. Web,
desktop, and CLI receive byte-identical readiness facts for the same
environment.

### S2 — The setup server and the web wizard

The primary surface (D7, D9). A loopback-only, token-authenticated server
serves the wizard; `vak setup` starts it and hands over the URL. The wizard
implements every step in Part IV, to the design contract in D10, with the
layer-targeting table in Part VI as its write contract.

Includes the non-technical-user pass end to end: copy, progressive
disclosure, failure sentences, mobile layout, and the Step 7 approval loop
demonstrated live.

**Exit:** a clean machine reaches a first receipt entirely through the
browser, on macOS and on a headless Linux box over a tunnel. Someone who
has never used the product completes it without reading a doc.

### S3 — `vak setup --terminal`

The same state machine as prompts, for anyone with no browser.
`--non-interactive` takes every choice from the environment
(`VAK_SETUP_PROVIDER`, `_MODEL`, `_POSTURE`, `_SEED`, `_ACTIVATE`,
`_FIRST_TASK`) and **fails on any missing choice**. Credentials come from
the environment or stdin, never argv — a key in a command line is in the
shell history, in `ps`, and in every process listing on the machine.

Menus print only when a terminal can answer them: offering a numbered list
to a pipe and then refusing buries the real error in noise.

**Exit:** clean machine to first receipt from the terminal alone.
Re-running is idempotent. A piped stdin with a missing choice refuses
rather than assuming.

### S4 — Desktop adopts the wizard

`ProjectGate` and `SetupCard` are deleted (Tier 1 item 8). The desktop
opens the same wizard the browser does, served by the backend it already
embeds.

Folder selection stops implying trust, which was a real hole rather than a
tidiness point: the desktop booted every picked folder with
`Core::new_with_trust(cwd, true)` and merged that project's `.env` into the
process environment, so opening a repository granted whatever its
`.vak/config.toml` asked for. Selection and consent are now two decisions —
a folder that asks for nothing privileged opens with no prompt, and one
that does gets a review naming exactly what it wants, read as text without
loading it. Reopening a remembered project uses the decision already on
record; opening safely is the absence of a decision and writes nothing.

**Exit:** desktop and web produce identical effective config from
equivalent choices, because they are the same code.

### S5 — The durable state registry

`StateEntry`/`StateRegistry` in `vak-core` (VII.2), populated from the
verified inventory. `--purge`'s preserve-allowlist, `vak_core::backup`'s
coverage, and the upgrade gate all read it instead of each carrying a
half-list. Both enforcement tests land here: no unregistered writes, and
purge matches the registry.

**Exit:** a durable file that nobody declared fails the build — verified by
temporarily writing one and watching the test fail, because a guard that
has never failed is a guard nobody has checked.

Landed with one finding: `vak_core::backup` hardcoded five directories and
four files, so everything added to the data home since — `gateway/` with
the whole channel allowlist, `operations/` with the incident ledger,
`inbox.jsonl`, `learning/` — was silently outside every backup ever taken.
It reads the registry now.

### S6 — The upgrade gate

The CI job in VII.5: install the previous release, drive the configured
fixture, snapshot, update to HEAD, assert per-entry survival, assert setup
re-runs nothing, then repeat downgrading so the older binary still reads
what the newer one wrote. Unavailable-is-failure throughout.

Also lands the non-destructive-update machinery it exists to protect:
pre-update snapshot, state entries in the install transaction, and
seed-digest tracking so an edited seed file is never clobbered.

**Exit:** the promises in VII.3 and VII.4 are properties of the build. This
is the phase that makes "we do not do this again" true rather than
intended — nothing after it can quietly break an existing install.

Shipped as `scripts/upgrade-gate.sh`, run by CI on every push. The
comparison rules live in `vak_core::state` beside the registry and are
applied by `vak self state --verify`, so a shell script and a library
cannot drift apart about what an update may do.

`--previous` defaults to the current build until a real prior release
exists. That is not a tautology: with both legs the same binary it still
proves install and update do not themselves disturb state, which is the
regression most likely to happen. Point it at a published artifact and the
same script becomes a true cross-version gate.

Verified by injection, not by watching it pass: rewriting
`security-events.jsonl` between the snapshot and the update makes the gate
fail with `contents changed (37 bytes → 18 bytes)`.

### S7 — Bundle and package

Bundle materialization moves into the release. DMG for macOS, tarball for
Linux, `install.sh` covering both. The DMG window and the docs state the
unsigned-first-open sequence in plain words (D2).

**Exit:** a person with no Rust toolchain installs from a downloaded
artifact on macOS and Linux and completes setup in a browser.

The bundle needed no new implementation: `self install --prefix …/Vakyartha.app`
already materializes one, so building it in the release uses the same code
path an install uses and the artifact cannot diverge from the installed
tree. What changed is *when* — at release time, so there is something a
notarization service could accept.

`hdiutil -exclude` does not reliably keep the installer's own scratch out
of the image, so the DMG is built from a staging root that contains exactly
`Vakyartha.app` and an `Applications` symlink — verified by mounting the produced
image and listing it.

### S8 — Release pipeline and evidence

Matrix release workflow, `--platform-key`, merged feed, integrity manifest,
signed feed, SBOM, `cargo audit`/`cargo deny` with unavailable-is-failure,
`dist/` pruning (Tier 3 item 20).

**Exit:** a release is produced by CI from a tag, describes every platform,
and is verifiable from the artifacts rather than from the build config.

`scripts/integrity-manifest.py` and `scripts/merge-feeds.py` both fail for
the right reasons, verified against fixtures: a tampered artifact is caught
as digest drift, and two legs claiming one platform key are refused rather
than silently overwriting each other.

Workflow inputs reach the shell through `env:` rather than `${{ }}`
interpolation inside `run:` — an input expanded into a command line is a
command-injection vector even when only people with write access can
dispatch it.

### S9 — Uninstall parity and the clean-slate cut

- Full-clean `--purge` (D3, Part VI): preserve-allowlist not delete-list,
  symlink refusal, scoped removal of our own units only, the
  `/usr/local/bin/vak` symlink reversed, and a filesystem-level test
  asserting **what survived**.
- Tier 2 legacy: delete the data-shape tolerances, with the binary
  refusing an old ledger by explicit message rather than serde error, and
  the release notes saying so. **Gated on the explicit yes called for in
  Part VII.**

**Exit:** install → setup → uninstall --purge → install leaves the second
install indistinguishable from a first install on a new machine, proven by
test.

### Verifying without the things we do not have

Two gaps had workarounds worth taking rather than waiting on.

**Signing.** With no Developer ID the release signs the bundle **ad-hoc**
(`codesign --sign -`). That is not a distribution signature — Gatekeeper
still refuses a double-click — but it is a real seal: the bundle becomes
internally consistent and modification after signing is detectable. It also
exercises the whole signing step, so the only change when a certificate
arrives is the identity. `VAK_SIGN_IDENTITY` supplies one.

The verifier reports three distinct states rather than two, because
collapsing ad-hoc into "unsigned" hides that the seal works and collapsing
it into "signed" claims a distribution signature we do not have. Running it
found a defect in itself: a **tampered** bundle was reported identically to
an honestly unsigned one and exited 0. A present-but-invalid signature is
now a hard failure — that is precisely what a seal exists to detect.

**Linux.** `scripts/linux-check.sh` runs the whole install and setup path
in a container, because half the supported platforms cannot be tested on
the machine most of this is written on. It found that `vak-ops` **did not
compile on Linux at all**: the `#[cfg(not(target_os = "macos"))]` branches
called a free `systemd_unit(name)` that never existed, and S1's
`Option`-returning accessors were only fixed on the macOS side.

Because Linux is headless, the check exercises *both* surfaces the way an
operator would: `vak setup --print-url`, then the projection and the
console over HTTP, and a check that an unauthenticated request is refused
even on loopback.

### S10 — Sign and notarize — **blocked on a Developer ID**

The half that does not need a certificate is done.
`scripts/verify-macos-release.sh` walks the **produced bundle** — executable
architecture, every dylib dependency `@`-relative or system-owned, the
signing authority, the Gatekeeper assessment, and a stapled ticket — and it
runs in the release today, reporting honestly that the artifact is
unsigned. That is invariant 15's actual requirement: signing is verified
from the artifact, never asserted by the build config. The dylib check
already earns its place: an absolute path into a build machine's Homebrew
tree works there and nowhere else, and reaches a user as "it just doesn't
open".

What is blocked is the certificate itself. `--require-signed` turns the
same script into a release gate the moment a Developer ID exists in CI
secrets; until then the honest artifact is an unsigned one that says so, in
the DMG and in the docs (D2).

**Exit, when unblocked:** the DMG opens with no warning, `--require-signed`
is on in the release workflow, and D2's unsigned notice is deleted rather
than left to rot.

### S11 — Windows — **not a packaging task, and not started**

Deliberately not attempted. Windows is missing three things that have
nothing to do with producing an `.msi`:

- **Paths.** `crates/vak-config` resolves a data home, cache home, logs
  directory, and Shared layer for macOS and Linux only.
- **A service backend.** `crates/vak-ops` speaks launchd and systemd. There
  is no Windows service implementation, so nothing could be activated.
- **A sandbox backend.** Seatbelt and Landlock only. An `.msi` over a
  binary with no containment would ship a fail-*open* product, which
  contradicts `docs/design/24-agent-security.md` and every restricted mode
  in `docs/design/08-permissions.md`.

Packaging it before those exist would produce something installable that is
less safe than the product it claims to be. Until they exist the supported
answer is **macOS and Linux**, said plainly in `README.md` rather than
implied by silence.

The one part already portable is the wizard: on Windows, first run would be
the same page (D7).
