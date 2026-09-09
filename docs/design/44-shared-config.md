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

Shared / Global Configuration
==============================

## Goal

A single configuration contract that spans every surface: the CLI, the
desktop app, the server, and channel bridges. There is one base layer
(User/Shared) and one per-surface layer (Project), composed with explicit
inherit-or-replace semantics. No surface may diverge (rule 27: configuration
has one contract across every surface).

## Layer Stack

```
User / Shared   (~/vak-home/.vak/config.toml + ~/vak-home/.env)
     │
     ├── inherited by every workspace ───┐
     │                                   │
Project    (.vak/config.toml + .env)     │ override or add
     │                                   │ inherit (narrower-wins)
Session / Task / Bot / Chat / Pin ────────┘
```

### Resolution Order (narrowest-wins)

1. **CLI flags** — scoped overrides, never global.
2. **Task/session pins** — applied at session creation.
3. **Subagent pins** — scoped to the subagent's lifetime.
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
process environment  >  project .env  >  shared .env  >  operator prompt
```

- Real environment variables always take precedence over `.env` files
  (rule 8).
- A project secret must never enter a process-global override map where
  another pooled workspace could observe it (rule 27). It is resolved
  per-workspace at `Core::new`.
- `Core::set_provider_key` / `Core::remove_provider_key` own the full
  lifecycle: setting a key invalidates the cached provider client and the
  discovered-model cache (rule 8).

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
