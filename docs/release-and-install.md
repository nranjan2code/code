# Release, distribution, and install

The practical runbook. `docs/design/32-release-engineering.md` owns the
installed lifecycle contract and
`docs/design/46-stabilization-install-and-onboarding.md` owns the design;
this is what you actually run, and what a person on each platform actually
gets.

## Platform support

| Platform | Supported | Install | Services | Sandbox |
|---|---|---|---|---|
| macOS (aarch64, x86_64) | yes | `.dmg`, `install.sh` | launchd | Seatbelt |
| Linux (x86_64, aarch64) | yes | `.tar.gz`, `install.sh` | systemd (user) | Landlock |
| Windows | **no** | — | — | — |

Windows is not a packaging gap. Paths (`crates/vak-config`), service
management (`crates/vak-ops` speaks launchd and systemd only), and
sandboxing (Seatbelt and Landlock only) are all two-platform. An installer
over a binary with no containment backend would ship a fail-*open* product,
contradicting `docs/design/24-agent-security.md` and every restricted mode
in `docs/design/08-permissions.md`. Until those three exist the honest
answer is macOS and Linux.

The one part already portable is the wizard: it is a web page served by the
backend, so on Windows first run would be the same page.

## Installing

### macOS

Download `Vakyartha-<version>-<arch>.dmg`, open it, drag `Vakyartha.app` to
Applications.

**The build is not yet signed by a Developer ID.** macOS will refuse a
double-click. Right-click `Vakyartha.app` → Open → confirm, once; every launch
afterwards is normal. `FIRST-OPEN-ON-MACOS.txt` ships beside the DMG saying
so, because meeting Gatekeeper's refusal with no warning reads as a broken
download.

The bundle *is* signed ad-hoc, which is not a distribution signature but is
a real seal: modification after signing is detectable
(`scripts/verify-macos-release.sh`).

### Linux

```bash
curl -fsSL https://get.vak.dev/install.sh | sh
```

Or download `vak-<version>-<arch>.tar.gz` and run `vak self install`.

The bootstrap script is POSIX `sh` (it runs under dash), verifies every
artifact's digest *before* placing anything, and then hands off to
`vak self install` — it never writes the install prefix itself, so
`status`, `verify`, `update`, and `uninstall` can all see the result
(doc 32 invariant 11).

### Then, on either

```bash
vak setup
```

Nothing runs until setup says so. Install places and verifies components;
it starts no service, creates no workspace, and seeds nothing.

## The lifecycle

| Command | Does |
|---|---|
| `vak self install` | places components, verifies digests, writes the manifest. Starts nothing |
| `vak setup` | the guided first run — workspace, trust, provider, route, posture, seeds, integrations, channels, activation, first task |
| `vak setup --terminal` | the same flow as prompts, no browser |
| `vak setup status [--json]` | what is configured and what is owed, with one repair each |
| `vak self status` | install and service state |
| `vak self verify` | check the install against its manifest |
| `vak self update` | pull a newer release, verify each artifact, replace transactionally |
| `vak self services-sync` | reconcile services with configuration |
| `vak self state [--verify S]` | digest declared durable state; check it against a snapshot |
| `vak self uninstall` | remove the install; keep configuration and data |
| `vak self uninstall --purge` | also remove all declared state — a genuine clean slate |

### Setup is the only thing that activates

Creating a bot, storing its token, or changing a setting writes
configuration and nothing else. Registering an OS service is a separate,
explicit act: setup's activation step, `vak self services-sync`, or
`POST /ops/services/activate`. A configured-but-not-activated bot is a
normal state, and `vak setup status` reports it as one.

The exception is revocation, which takes effect immediately without
orchestration: a bridge watches its own credential, so clearing a token
stops it authenticating within a poll cycle.

## Cutting a release

### Locally

```bash
scripts/release.sh --base-url https://get.vak.dev
```

Gates first — version singularity, both frontends rebuilt and matching
their sources, `cargo fmt`, `clippy -D warnings`, the test suite, a clean
tree, and an unused tag — then builds, then produces artifacts under
`dist/<version>/`:

| Artifact | What |
|---|---|
| `vak`, `vak-desktop`, `vak-delivery-worker` | the components |
| `Vakyartha-<version>-<arch>.dmg` | macOS app, signed ad-hoc |
| `vak-<version>-<arch>.tar.gz` | Linux archive |
| `install.sh` | the bootstrap script, travelling with the release it installs |
| `SHA256SUMS` | per-file checksums |
| `integrity-manifest.json` | tamper-evidence over the whole *set* |
| `release.json` | the feed `vak self update` consumes |

The macOS bundle is built **here**, not on the installing machine. A bundle
assembled at install time could never be signed or notarized, because you
cannot notarize something that does not exist until install. It is produced
by `self install --prefix …/Vakyartha.app`, the same code path an install uses,
so the artifact and the installed tree cannot diverge.

### In CI

`.github/workflows/release.yml` runs on a `v*` tag: a matrix over
macos-14, macos-13, and ubuntu, each leg declaring what it built with
`--platform-key`, then a merge leg combining every leg's feed into one.

`scripts/release.sh` derived one platform key from `uname`, so a laptop
release could only ever describe the machine that ran it — every other
platform was absent from the feed and `self update` had nothing to offer
those installs.

## Evidence

| Check | Tool | Answers |
|---|---|---|
| per-file integrity | `SHA256SUMS` | was this file corrupted? |
| set integrity | `scripts/integrity-manifest.py --verify` | was an artifact added, removed, or swapped? |
| macOS artifact | `scripts/verify-macos-release.sh` | architecture, dylib references, signature, Gatekeeper, stapled ticket |
| advisories | `cargo audit` | any known-vulnerable dependency? |
| dependency inventory | `cargo cyclonedx` | what is actually in this build? |

Per-file checksums prove a file was not corrupted; they do not prove the
*set* was not edited. The integrity manifest names each artifact with its
byte length and digest, plus a digest over its own canonical payload, and
the verifier rejects a missing, extra, duplicated, or path-escaping entry.

**An unavailable check is a failure, not a pass.** A missing audit tool
fails the job rather than silently skipping the gate it was there to
enforce.

Signing is verified **from the produced artifact**, never asserted by the
build config. `verify-macos-release.sh` reports three distinct states —
Developer ID, ad-hoc, unsigned — and treats a present-but-invalid signature
as a hard failure, because detecting exactly that is what a seal is for.

## Not breaking existing installs

`scripts/upgrade-gate.sh` installs a prior artifact, configures it, snapshots
every file the durable state registry declares, updates to this build, and
asserts per-entry survival — then repeats downgrading, because the leg
people forget is whether the older binary can still read what the newer one
wrote.

The rules live in `vak_core::state` beside the registry and are applied by
`vak self state --verify`, so a shell script and a library cannot drift
apart about what an update may do:

| Kind | An update may |
|---|---|
| Ledger | nothing — byte-identical |
| Config | add fields, never remove or redefine one |
| Derived | anything; it is rebuilt |

`--previous` defaults to the current build until a release exists to point
at. That still proves install and update do not themselves disturb state;
one flag makes it a true cross-version gate.

## End-to-end checks

Two scripts, one per supported platform, asserting the same properties.
Both run against an isolated home and prefix — they drive destructive
lifecycle commands on purpose and must never touch real state.

```bash
scripts/macos-check.sh --with-model <ollama-model>
scripts/linux-check.sh --with-model <ollama-model>
```

Each asserts: install places and starts nothing; the install verifies
against its manifest; setup refuses a missing choice and completes when
told; CLI and HTTP report identical state for every core step, because they
render one derivation; the wizard serves and refuses unauthenticated
requests; the guided first task is capped ReadOnly regardless of the
workspace posture; a real turn produces a real dispatch receipt; a bot is
configured *without* being activated and its token never comes back; the
integration catalog is offered as peers with no key returned; an update
preserves everything the registry protects; and a purge leaves nothing.

The Linux check reaches the host's Ollama at `host.docker.internal`, so the
Linux path is proven to dispatch rather than merely to start.

Credentials come from the environment and are never printed:

| Variable | Exercises |
|---|---|
| `VAK_CHECK_BOT_TOKEN` | the channel path, with a real bot |
| `VAK_CHECK_TAVILY_KEY` | the integration path, with a real key |

Both steps skip when unset, so the scripts run without secrets and contain
none.

## Testing Linux from a Mac

```bash
scripts/linux-check.sh          # pass/fail: install, setup, web, purge
scripts/linux-check.sh --serve  # leave the Linux wizard open in your browser
```

Half the supported platforms cannot be tested on the machine most of this
is written on: different paths, a different service backend, a different
sandbox, and `install.sh` running under dash. The container check found
that `vak-ops` did not compile on Linux at all.

Because Linux is headless, `--serve` is how its web surface gets looked at
rather than only asserted about: it prints a `http://127.0.0.1:7788/…` URL
for a Mac browser. The setup server still binds loopback only *inside* the
container; a relay forwards the published port, so the exposure is explicit
and dies with the container.

## Uninstalling

```bash
vak self uninstall            # remove the install, keep everything else
vak self uninstall --purge    # remove all declared state as well
```

`--purge` derives its targets from the state registry and names every path
before deleting — "delete everything?" is a question people answer without
reading. It removes the Shared configuration layer, stored keys, sessions,
skills, and plugins, so the next install is a genuine first run.

It never touches a project's own `.vak` directory inside your repositories:
those are your files. A symlinked root is refused rather than followed.

## Versions before 2.0.0

Not supported and not upgradable in place. State written by an earlier
version is refused with one message naming the file, the version found, the
baseline expected, and the single command that resolves it. Nothing is
partially read and nothing is migrated (`AGENTS.md` invariant 29).

```
vak self uninstall --purge     then install 2.0.0 and run setup
```
