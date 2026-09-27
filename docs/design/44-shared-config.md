Shared / Global Configuration
==============================

Status: implemented in 3.0.24

## AWS Bedrock endpoint management

Bedrock is a first-class provider in the shared/project configuration layers.
The provider/model route is persisted atomically using the same inheritance
and provenance rules as other providers. The bearer secret is resolved from
the canonical secret chain as `AWS_BEARER_TOKEN_BEDROCK`; it is never stored
in the TOML route or returned by the admin API.

Discovery uses the configured Bedrock Mantle endpoint. When the host has a
standard AWS SDK credential chain, the provider response also reports native
Bedrock availability per model: agreement, authorization, entitlement, and
regional status, plus an `invokable` result. Status is shown in Settings and
models not confirmed fully available are disabled. A failed control-plane
check is surfaced as an error rather than treated as authorization.

This split supports headless vak hosted in AWS: Mantle inference can use the
scoped bearer key, while availability checks use an IAM role, SSO/web
identity, or another standard AWS credential source. Results are cached for
five minutes, with the credential/endpoint identity included in the cache key.

## Goal

A single configuration contract that spans every surface: the CLI, the
desktop app, the server, and channel bridges. There is one base layer
(User/Shared) and one per-surface layer (Project), composed with explicit
inherit-or-replace semantics. No surface may diverge (rule 27: configuration
has one contract across every surface).

## Layer Stack

```
User / Shared   (~/vak-home/.vak/config.toml + shared secret scope)
     │
     ├── inherited by every workspace ───┐
     │                                   │
Project    (.vak/config.toml + project secret scope) │ override or add
     │                                   │ inherit (narrower-wins)
Session / Task / Bot / Chat / Pin ────────┘
```

Secret scopes are resolved through `vak_config::credentials`, not read as
literal `.env` files — see "Secrets Chain" below.

### Resolution Order (narrowest-wins)

1. **CLI flags** — scoped overrides, never global.
2. **Task/session pins** — applied at session creation.
3. **Worker pins** — scoped to the worker's lifetime.
4. **Chat policy** — bot → chat → workspace (rule 23).
5. **Bot routing** — per-bot route override or workspace default.
6. **Workspace** — the project config (`.vak/config.toml`).
7. **User/Shared** — the base layer (`~/vak-home/.vak/config.toml`).

## Trust Model

A project config is **untrusted** until the workspace is trusted
(`trust_project = true` set by the operator). When untrusted:

- Privileged keys are stripped from the effective config. The
  `PRIVILEGED_KEYS_NOTICE` constant in `crates/vak-config/src/lib.rs`
  enumerates them: `[server]`, `[gateway]`, `[finops]`, `[automation]`,
  `[tools]`, `[route]`.
- The `fc.server = ServerSettings::default()` fallback applies
  (rule 27: untrusted projects cannot choose an interface or relax host
  checks).

See `load_with_trust` in `crates/vak-config/src/lib.rs` for the stripping
logic.

## Secrets Chain

Secrets follow the same lookup chain but stay outside TOML:

```
process environment  >  project secret  >  shared secret  >  operator prompt
```

- Real environment variables always take precedence over stored secrets
  (rule 8).
- A project secret must never enter a process-global override map where
  another pooled workspace could observe it (rule 27). It is resolved
  per-workspace at `Core::new`.
- `Core::set_provider_key` / `Core::remove_provider_key` own the full
  lifecycle: setting a key invalidates the cached provider client and the
  discovered-model cache (rule 8).

**Storage backend.** "Project secret" and "shared secret" above no longer
name a literal `.env` file — they name a *scope*, resolved through
`vak_config::credentials` (`crates/vak-config/src/credentials.rs`) to
whichever `CredentialStore` this host uses:

- **OS-native** (macOS Keychain or Linux Secret Service on supported platforms,
  via the `keyring` crate) — used whenever a round-trip probe
  against it succeeds.
- **Encrypted-file fallback** — an AES-256-GCM-encrypted file under the
  shared data home (`credentials.enc`, keyed by a separate 0600
  `.credential_key` file), used whenever no OS secret service is reachable.
  This is the common case for headless Linux (servers, containers, CI with
  no D-Bus session) and is a first-class backend, not a degraded stand-in
  for `.env` — a copy of `credentials.enc` alone reveals nothing without
  the key file.
- **Process environment** — read-only, every platform, never persisted;
  always wins per the precedence above (CI/ephemeral containers).

Selection is automatic per host, not configured. `vak_config::upsert_env_file`
/ `read_env_file_var` / `remove_env_file_key` keep their existing
signatures — callers pass a scope hint (the directory that used to hold a
literal `.env`) rather than a literal file, so `Core`'s ~15 call sites and
`vak-desktop`'s startup env-loading needed no changes beyond this module's
internals. A plaintext `.env` file is never written by this codebase.

The OS-native backend has no portable "list everything for this service"
API, so bulk operations (`load_env_file`, `replace_env_files` — used to
seed the legacy process-env cache for raw `std::env::var` consumers) only
have effect on the encrypted-file backend; point lookups via
`read_env_file_var` work identically on both. A small non-secret presence
index (`credential_index.json`, scope+var names only, never values) answers
"does this scope have anything stored" (`credentials::scope_has_any`) for
trust decisions that used to check literal file existence, since the
OS-native backend can't answer that by enumeration either.

## Permission Mode Composition

`PermissionMode::capped_by` (rule 17) returns the least-privilecive mode
when composing layers. It is universal in the deny direction — a narrower
scope can only restrict, never expand, the effective mode.

```rust
let effective = workspace_mode.capped_by(project_mode).capped_by(session_mode);
```

## Cross-Surface Consistency

- `/config`, `/providers`, `/health`, session transcripts, and admin
  snapshots all report effective values and their source (rule 17).
- A GET used to seed a PUT returns that exact layer, never the merged
  projection (rule 21).
- Permission-mode changes revoke old capability before the new mode is
  reported (rule 11).

## Liveness

"One contract across every surface" (rule 27, above) covers *what* the
layers compose to. It does not by itself cover *when* a surface learns
that composition changed — a config write from one process (`vak setup`,
another client) was previously invisible to an already-running `Core` or
UI until it was evicted/restarted, because `RouteSelection` was resolved
once and cached forever.

- `Core` tracks a cheap, stat-only fingerprint
  (`vak_config::config_fingerprint`: mtime+size of the global and project
  `config.toml`) alongside its cached route. `effective_route()` compares
  the fingerprint on every call and, on a mismatch, re-derives via the
  already-existing `refresh_persisted_route()` — which already respected
  `runtime_pinned` (CLI/task/heartbeat/worker scopes correctly do **not**
  pick up a global edit mid-run). This was a wiring gap, not missing
  logic: `refresh_persisted_route()` and `CorePool::invalidate_pooled()`
  both predate this fix and are unchanged.
- Live UIs (desktop, web) subscribe to the server's existing
  `EventHub`/`SystemEvent::ConfigChanged` stream (already emitted by every
  config-write handler) over the admin SSE endpoint (`/admin/api/events`)
  via `api.openConfigEvents()`, and refetch on receipt — see
  `crates/vak-client-ui/src/components/Settings.tsx`. No new server-side
  event plumbing was needed; only the client subscription was missing.
- Scope: this covers one server process's own `Core`/pooled cores and the
  UIs connected to it. A multi-gateway/distributed deployment where two
  separate `vak-server` processes must agree is a different problem —
  `vak-bus`'s NATS backend is the natural fit for that (a `ConfigChanged`
  `Subject`), but is not implemented; today `vak-bus` is used only for
  intra-server session-event streaming and has no config-related subject.
