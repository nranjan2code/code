# 37 — Distribution: one installer, every platform

Status: proposed design. Result of a study of the sibling project
**vakyartha** v5.30.136 (`/Users/nisheethranjan/Projects/vakyartha`),
August 2026 — the second such study; the first produced
`docs/design/27-vakyartha-adoption.md`, which took loop and reliability
mechanisms. This one takes *distribution* mechanisms only.

Companion docs: `docs/design/32-release-engineering.md` owns the installed
lifecycle (install → verify → status → update → uninstall);
`docs/design/36-first-run-onboarding.md` owns what happens after the bits
land. This doc owns the step neither covers: **how the bits get onto a
machine that has never seen vak, on any platform, without a compiler.**

## The honest comparison

Do not start from "vakyartha does packaging well, copy it." It does one
thing extremely well and two things not at all, and the parts we are
weakest at are not the parts it solves.

| Axis | vakyartha | vak (today) | Who wins |
|---|---|---|---|
| Artifact a user can actually double-click | signed + notarized `.dmg` and `.app.zip` | none — `git clone` + `cargo build` | **vakyartha** |
| Code signing / notarization / Gatekeeper | required, and *verified against the built artifact* | absent | **vakyartha** |
| Tamper-evident manifest over the whole distribution | `integrity-manifest.json` + `manifestSha256`, independently re-verified | `SHA256SUMS` per binary only | **vakyartha** |
| Dependency inventory (SBOM) + advisory gate | CycloneDX 1.6 from both lockfiles; `pnpm audit` + `cargo audit`, where an *unavailable* audit tool is a failure | neither | **vakyartha** |
| Version singularity | one source of truth fanned out to ~20 files by a bump script that must keep being taught new files | one stamp, nothing mirrors it, and a checker proves no second stamp reappeared | **vak** |
| In-app / in-CLI update | none — the user re-downloads a DMG | `self update`: feed, per-artifact digest, semver ordering, staged transaction, rollback | **vak** |
| Uninstall | "Clean local data" with a tested preserve-allowlist; no binary removal | `self uninstall` reverses install exactly; `--purge` for user data | **vak** (mechanism), **vakyartha** (data-preservation rigor) |
| Cross-platform | refuses to build on anything but Apple Silicon, deliberately | macOS + Linux; Windows absent everywhere (paths, services, sandbox) | neither |

The conclusion that matters: **vakyartha cannot tell us how to ship on
every platform, because it deliberately ships on one.** What it can tell
us is what a *credible* artifact looks like on the one platform it
targets — signed, notarized, inventoried, and verified by evidence taken
from the artifact rather than inferred from the build config. Our
lifecycle machinery is better than theirs and mostly idle, because
nothing ever gets far enough to use it: there is no first install that
isn't a compile.

## Invariants

These extend doc 32's list; they do not replace any of it.

13. **A first install requires no toolchain.** The supported path onto a
    clean machine is one command or one download. `cargo install` stays
    available for contributors and stops being the headline in
    `README.md`.
14. **One writer still owns the install root.** Doc 32 invariant 11
    survives packaging: a `.dmg`, `.deb`, or bootstrap script *stages*
    components and then hands off to `vak self install`, which writes the
    manifest. No package manager's postinst may lay files into the prefix
    behind the installer's back — that is precisely the failure doc 32
    already records, arriving through a new door.
15. **Signing is verified from the artifact, not asserted by the build.**
    A release is signed and notarized only after a verifier has walked the
    produced bundle: every executable payload the expected architecture,
    every dylib dependency `@`-relative or system-owned, a Developer ID
    authority in the signing record, Gatekeeper assessment passing, and a
    stapled ticket validating. Adopted wholesale from
    vakyartha's macOS release verifier (their scripts/verify-macos-release.sh).
16. **The distribution is tamper-evident as a set.** Per-file checksums
    prove a file wasn't corrupted; they do not prove the *set* wasn't
    edited. Every release directory carries a manifest listing each
    artifact's name, exact byte length, and SHA-256, plus a digest over the
    manifest's own canonical payload — and a verifier that rejects a
    missing, extra, duplicated, or path-escaping entry, size or digest
    drift, and mutation of the payload.
17. **The update feed is authenticated, not merely fetched over TLS.**
    Today `self update` trusts whatever `release.json` the URL returns and
    verifies artifacts against digests *that same document* supplied.
    That defends against a corrupted download, not against a controlled
    origin. The feed carries a detached signature over its canonical bytes;
    the public key is compiled into the binary; an unsigned or
    wrongly-signed feed is refused before any digest is consulted.
18. **An unavailable check is a failure, not a pass.** Vakyartha's rule,
    stated because the opposite is the natural coding accident: an audit
    tool that isn't installed, an advisory service that times out, a
    notarization service that errors — each fails the release. Silence is
    never evidence.
19. **Every platform key in the feed is produced by CI, not a laptop.**
    `scripts/release.sh` currently derives one platform key from `uname`,
    so a release can only ever describe the machine that ran it. A release
    is a matrix build; the feed is assembled from every leg.

## Target shape

Three layers, each one an entry point for a different person. All three
converge on `vak self install`.

### Layer 0 — the bootstrap script (this is "the single installer")

    curl -fsSL https://get.vak.dev/install.sh | sh          # macOS, Linux
    irm https://get.vak.dev/install.ps1 | iex               # Windows, later

It detects OS and architecture, fetches the signed feed, verifies the
signature, downloads that platform's components, verifies each digest,
places them in a staging directory, and execs the freshly downloaded
`vak self install --prefix …`. It never writes the prefix itself
(invariant 14), so status, verify, update, and uninstall all see the
result. Re-running it is exactly `self update`.

This is the answer to "one installer on any platform": one URL, one
command, a per-platform payload behind it — not one binary artifact that
runs everywhere, which does not exist for a signed native app with a
webview.

### Layer 1 — native artifacts, for people who want them

| Platform | Artifact | Notes |
|---|---|---|
| macOS | signed + notarized `.dmg` containing `Vak.app` | the `.app` is already ours: `crates/vak/src/install/bundle.rs` writes `Info.plist` and stages the desktop frontend into `Contents/Resources`. Today it is only ever built *on the installing machine*; the release must build and sign it instead. |
| Linux | `.tar.gz` (primary), `.deb` + `.rpm` (secondary) | package payload unpacks to a staging dir; postinst calls `vak self install`. No systemd unit ships in the package — units stay generated by `self services-sync` (doc 32 invariant 3). |
| Windows | `.msi` | blocked on real platform work, below. |

### Layer 2 — setup, which is not install

Unchanged from `docs/design/36-first-run-onboarding.md`: install places and
verifies; setup chooses a workspace, resolves trust, connects a provider,
freezes a route, picks a permission posture, and only then optionally
activates durable services. Packaging must not quietly re-merge the two —
a `.pkg` postinst that starts a gateway is the same mistake as
`self install` bootstrapping one.

## What is actually missing today

Ordered by what blocks the next one.

1. **CI does not run.** `.github/workflows/ci.yml` had a mis-indented step
   in the `linux` job, which makes the whole workflow file unparseable —
   so neither job has been running. Fixed alongside this doc; noted here
   because every gate below assumes CI works.
2. **No release workflow exists.** `.github/workflows/` has `ci.yml` and
   `chaos.yml`. Releases are laptop-run and single-platform (invariant 19).
3. **No signing identity, anywhere.** Neither `crates/vak-desktop/tauri.conf.json`
   nor `scripts/release.sh` mentions signing. A downloaded unsigned binary
   is quarantined by Gatekeeper; a downloaded unsigned `.app` is refused
   outright. Until this is solved, Layer 0 and Layer 1 on macOS both
   produce something the user must right-click-open past a scare dialog —
   which is not a product.
4. **The release ships bare binaries, not the `.app`.** `scripts/release.sh`
   copies `target/release/vak-desktop` and friends; the bundle is
   assembled later, on the user's machine, by the installer. That was a
   reasonable way to avoid duplicating bundle logic, and it becomes wrong
   the moment the bundle has to be signed and notarized — you cannot
   notarize something that does not exist until install time.
5. **No SBOM, no advisory gate.** No `cargo audit` or `cargo deny` in CI or
   in the release gate; nothing emits a dependency inventory.
6. **No integrity manifest** (invariant 16) and **no feed signature**
   (invariant 17).
7. **`README.md` still leads with cloning and compiling** — already
   recorded in doc 36's current-state audit, restated here because Layer 0
   is what replaces it.
8. **Windows is not a packaging gap; it is a platform gap.** Paths
   (`crates/vak-config`), service management (`crates/vak-ops`: launchd and
   systemd only), and sandboxing (Seatbelt and Landlock only) are all
   two-platform. An `.msi` over a binary with no sandbox backend would ship
   a fail-*open* product, which contradicts the security posture in
   `docs/design/24-agent-security.md`. Say "macOS and Linux" honestly until
   the sandbox story exists.

## Phases

**P1 — make releases reproducible off a laptop.** Fix CI (done). Add a
release workflow: matrix over macos-14 (aarch64), macos-13 (x86_64),
ubuntu (x86_64, aarch64); each leg runs the existing gates from
`scripts/release.sh`, builds, and uploads its component set; a final leg
merges every platform key into one `release.json`. Extend
`scripts/release.sh` with a `--platform-key` override so a leg can declare
what it built rather than inferring it from `uname`.

**P2 — evidence.** Port vakyartha's three verifiers, adapted: a
macOS artifact verifier (invariant 15, near-direct port of theirs), a
distribution integrity manifest plus verifier (invariant 16), and an SBOM
generator over `Cargo.lock` and the two frontend lockfiles. Add
`cargo audit` and `cargo deny` to CI and to the release gate, with
tool-unavailable treated as failure (invariant 18). Their command-boundary
audit has a direct analogue worth building later: our Tauri `invoke`
surface in `crates/vak-desktop` deserves the same "every registered
command classified, every renderer call registered" gate.

**P3 — sign and notarize.** Apple Developer ID, secrets in CI, notarize and
staple the `.dmg` and the `.app`. Build the bundle *in the release* rather
than at install time: `crates/vak/src/install/bundle.rs` grows a
"materialize a bundle at this path" entry point that both the release
script and `self install` call, so there remains one implementation of
what a Vak bundle is. Linux signing is a detached signature over the
tarball plus the feed signature; there is no Gatekeeper analogue to satisfy.

**P4 — the bootstrap script and the feed.** Ship the install script and the
signed feed; host `release.json` at a stable URL so the passive `[update]`
check has a real default. Rewrite `README.md`'s quick start around it and
demote source builds to a contributor section. At this point re-running the
one-liner is an update, and doc 32's whole lifecycle is finally reachable
by someone who never cloned the repo.

**P5 — uninstall parity.** `self uninstall --purge` should adopt
vakyartha's discipline for the destructive half: an explicit *preserve*
allowlist rather than a delete list, refusal when the data root is a
symlink, scoped removal of only our own service units, and a filesystem-level
test that asserts what survived — not only what was removed. Ours is
already symmetric with install; this makes the data side auditable.

**P6 — Windows,** as its own project: paths, a service backend, and a real
sandbox backend before any `.msi`.

## What we are deliberately not importing

| vakyartha mechanism | Verdict | Reason |
|---|---|---|
| Multi-file version fan-out via a bump script | **reject** | our version singularity (doc 32 invariant 1) is strictly better; their `bump-version.ts` exists to manage a problem we do not have |
| Apple-Silicon-only build refusal | **reject** | correct for a macOS product, fatal for a CLI-first harness |
| "Re-download the DMG" as the update story | **reject** | `self update` is ours and is better |
| DSL/renderer-specific release checks | **reject** | different product surface |
| Executable release checklist as a document | **adopt in spirit** | ours belongs in the release script as gates, not in prose a human must remember to read |
