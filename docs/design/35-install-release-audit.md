# 35 — Install and release audit

Status: release-blocking audit, 2026-08-26. This document records observed
behaviour separately from the target contract. A checked box means the path is
implemented and exercised from a release artifact, not merely described.

## Product boundary

VakCoder ships as one required base and optional clients:

| Package | Owns | Must not own |
|---|---|---|
| `vakcoder-base` | CLI, gateway, embedded admin assets, workers, service lifecycle | Native UI, desktop-local agent server |
| `vakcoder-tui` | Terminal client | Agent state, scheduler, gateway |
| `vakcoder-desktop` | Native client and desktop integration | Agent state, scheduler, embedded competing gateway |
| channel addons | A bridge such as Telegram | Automatic activation without configuration |

The base must be fully usable with only headless commands and the browser admin
console. Installing or removing a client must never replace the base, move user
data, change the gateway token, or start a second state owner.

## Audit findings

### Release blockers found

1. ~~The `vakcoder` crate had an unconditional dependency on `vak-tui`.~~ Fixed:
   headless builds use `--no-default-features`, and `vakcoder-tui` is an
   independently installable client.
2. ~~The desktop started an ephemeral embedded `vak-server` per project.~~
   Fixed: it now discovers and handshakes with the local or configured base and
   no longer depends on core, server, delivery, or tool-worker crates.
3. ~~The macOS self-installer treated the base as an application bundle.~~
   Fixed: the base uses versioned program directories, a stable `current`
   activation, a durable install-root receipt, and a separate CLI launcher.
4. Windows is not implemented as a release target. `vak-config::paths` applies
   XDG paths to every non-macOS OS and `vak-ops` applies systemd to every
   non-macOS OS. No Windows service, package, update, or launch contract exists.
5. `self update` still trusts an unsigned manifest. Binary SHA-256 verification,
   staged version activation, and numeric semantic ordering are implemented;
   feed signing and automatic rollback remain before GA.
6. Candidate automation now builds and smokes base/TUI artifacts on macOS and
   Linux and builds the unsigned macOS desktop bundle. Signing, notarization,
   SBOM/provenance, Windows artifacts, and publication gates remain.

### High-priority drift

1. `build-install.sh` is a source-build helper for macOS desktop only. It is not
   an end-user installer and bypasses the managed base lifecycle.
2. `scripts/install_gateway_service.sh` built into `target/release`, wrote
   legacy `~/.vakcoder` paths, embedded environment values in units, and
   pointed services at the build tree. It is now a compatibility wrapper over
   the managed installer and generated `vak-ops` units.
3. `self install --prefix` is not durable: status, service sync, update, and
   uninstall return to the platform default instead of reading the selected
   prefix from an install receipt.
4. Base installation previously enabled Telegram automatically even when the
   user had not selected or configured it. The base installer now activates
   only the gateway; optional services remain explicit.
5. `tauri.conf.json` carried a stale 0.7.0 version despite the workspace being
   0.8.0. The duplicate stamp has been removed so Tauri derives the Cargo
   package version.
6. CI had malformed Linux step indentation, so documentation and Landlock
   checks were not represented by a valid job definition. This is corrected.

### Observed macOS migration fixture

The development Mac on 2026-08-26 had all of the following simultaneously
(no GUI process remained after the audit):

- `~/Applications/VakCoder.app` version 0.4.0;
- `/Applications/VakCoder.app` version 0.8.0;
- Cargo-installed `~/.cargo/bin/vakcoder` version 0.8.0;
- a managed release tree containing a 0.7.0 binary and a receipt with legacy
  `~/.vakcoder` paths;
- gateway, Telegram, and tray LaunchAgents executing from the system app;
- a healthy gateway and a stale tray process detected by `self status`.

The last item was a false positive: the tray started at 18:09 after its 0.8.0
binary was written at 18:07, while its plist was older. The detector compared
binary mtime to unit mtime rather than process start time. It now parses `ps
-o etime` and compares the live process start to the binary mtime.

The 0.4.0 user app and both 0.8.0 apps are ad-hoc signed and fail Gatekeeper
validation. The Cargo install database names `vakcoder 0.6.1` while its current
binary reports 0.8.0. The active gateway reports protocol 1 / version 0.8.0 and
runs from the system app bundle. These copies must remain untouched until the
new installer migration has a reviewed cleanup set.

The migration must discover this state, choose one current base, rewrite the
gateway unit, remove only obsolete program files after confirmation, and leave
the data home, token, sessions, memory, tasks, and logs untouched.

## Target filesystem and launch contract

Program files and user state are separate. The paths below are package-owned;
data paths remain those returned by `vak_config::paths`.

| OS | Base program files | CLI entry | Base service | Desktop |
|---|---|---|---|---|
| macOS | `~/Library/Application Support/vakcoder/runtime-bin/versions/<version>/` | `~/.local/bin/vakcoder` symlink to `current` | LaunchAgent `com.vakcoder.gateway` | `/Applications/VakCoder.app` or `~/Applications/VakCoder.app` |
| Linux | `$XDG_DATA_HOME/vakcoder/runtime-bin/versions/<version>/` | `~/.local/bin/vakcoder` symlink to `current` | `systemd --user` `vakcoder-gateway.service` | AppImage first; deb/rpm later |
| Windows | `%LOCALAPPDATA%\VakCoder\runtime-bin\versions\<version>\` | installer-managed user `PATH` | per-user Scheduled Task initially; Windows Service only if privileged install is added | signed MSIX |

Version directories make update rollback possible. `current` changes only after
the staged base passes `--version`, `doctor`, worker self-check, and a loopback
health probe. Service units always point through the stable `current` entry.

## Supported user scenarios

### A. Base only, first installation

1. User downloads one signed platform artifact or invokes the bootstrap
   installer.
2. Installer verifies signature and SHA-256, stages a version directory, writes
   the install receipt, exposes the CLI, and starts only the gateway.
3. First gateway boot creates the data home and token. The provider wizard is
   optional; Ollama or later configuration remains possible.
4. `vakcoder admin` opens the browser console; headless users may run
   `vakcoder exec`, `plan`, `doctor`, `backup`, and `self status` without TUI or
   desktop packages.

Acceptance: uninstalling all UI packages before and after this scenario does
not affect any command above.

### B. Install a UI later

- TUI package adds only `vakcoder-tui`. It discovers the local runtime receipt
  or a saved remote profile and connects over HTTP+SSE.
- Desktop package adds only the native app. On first launch it discovers the
  base; if none exists it explains how to install or connect to one. It never
  silently boots a second base.
- The tray is an optional native addon installed with `--with-tray`. It runs as
  `com.vakcoder.tray`, reads the same managed service state, and can open Admin;
  it is never enabled by a base-only install.
- Removing either client leaves the gateway and data unchanged.

### C. Opening each surface

| Surface | macOS | Linux | Windows |
|---|---|---|---|
| Admin | `vakcoder admin` or tray menu | `vakcoder admin` | `vakcoder admin` |
| TUI | `vakcoder-tui` | `vakcoder-tui` | `vakcoder-tui.exe` in Windows Terminal |
| Desktop | Finder/Spotlight or `open -a VakCoder` | application menu or AppImage | Start menu |

`vakcoder admin --print` is the non-GUI/server-safe form. The command reads the
live runtime receipt and sends the token in the URL fragment, which is exchanged
for an HttpOnly cookie and then removed by the admin client.

### D. Update and rollback

1. Passive checks notify only.
2. Explicit update downloads the platform's base artifact plus provenance,
   verifies signature/hash, stages it beside the current version, runs health
   checks, atomically switches `current`, and restarts the gateway.
3. If health fails, `current` is restored and the old process restarted.
4. Client packages update independently but enforce major-protocol
   compatibility through `/version`.
5. Two known-good base versions are retained; data is never rolled back by a
   program rollback.

## Platform verification matrix

Every release candidate must pass these artifact-level tests:

- clean base-only install; first boot; admin login; one mock-provider turn;
- install TUI later; connect; run and cancel a turn; uninstall TUI;
- install desktop later; connect to the same session set; uninstall desktop;
- update N-1 to N with a running gateway and verify PID/version convergence;
- injected corrupt download and failed health check both preserve N-1;
- uninstall keeps data; reinstall rediscovers it; purge requires explicit
  confirmation;
- paths containing spaces and non-ASCII characters;
- no provider or gateway secret in units, process arguments, logs, receipts,
  archives, or crash output.

macOS tests run on a clean user account and inspect launchd. Linux tests run in
both Docker (artifact/layout/CLI tests) and a VM or CI runner with a real user
systemd session (service lifecycle). Docker alone cannot prove `systemd --user`
login/linger behaviour. Windows tests require a Windows CI runner; Wine is not
an adequate service, MSIX, WebView2, or Terminal test.

## Delivery phases

1. **R1 — stop drift:** make CI valid, remove duplicate versions, stop implicit
   addon activation, deprecate build-tree service installation, and document
   the observed migration state.
2. **R2 — separable artifacts:** feature-split the base, add the standalone TUI
   client, and make desktop remote-only.
3. **R3 — managed installer:** implement durable receipts, stable entrypoints,
   explicit addon install/remove, and legacy discovery on macOS and Linux.
4. **R4 — trustworthy update:** signed checksummed manifests, semantic version
   handling, staged health checks, atomic activation, and rollback.
5. **R5 — release automation:** tagged matrix builds, SBOM/provenance, signing,
   release notes, artifact-level smoke tests, and publication gates.
6. **R6 — Windows:** native paths, Scheduled Task lifecycle, MSIX, Windows
   Terminal TUI, WebView2 desktop, signing, and Windows-runner scenarios.

R1–R3 are implemented in this change. The
isolated macOS artifact smoke passes; the same base install/uninstall plus
standalone TUI build passes in a Linux ARM64 Docker container. The full
workspace fmt, clippy-with-warnings-denied, and test gate passes.

## Development Mac migration result

The reviewed migration was applied on 2026-08-26. Base, TUI, and desktop report
0.9.0. Gateway and the explicitly selected Telegram bridge execute through the
managed `current` path. The old tray unit, 0.4.0 user app, 0.7.0 release tree,
Cargo-installed base, Cargo tray, and superseded 0.8.0 managed program version
were moved to Trash or uninstalled. `/Applications/VakCoder.app` is the only
desktop copy. No session, configuration, secret, memory, task, cache, or log
data was removed.

The base install/uninstall smoke passed in a disposable macOS home and a Linux
ARM64 Docker container with service management disabled. Full workspace fmt,
clippy with warnings denied, and tests passed after the 0.9.0 rebuild. The app
remains an unsigned development artifact; public macOS distribution is still
blocked on signing and notarization.

No public release should be called generally available until R1–R5 pass for
macOS and Linux. Windows must be labeled unsupported until R6 passes.
