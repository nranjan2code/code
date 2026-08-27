# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.9.x   | Yes       |
| < 0.9   | No        |

## Reporting a Vulnerability

If you discover a security vulnerability in vakcoder, please report it
responsibly:

1. **Do not** open a public GitHub issue for security vulnerabilities.
2. Email the maintainers at the address listed in `Cargo.toml` or open a
   **private** security advisory at
   <https://github.com/vakcoder/vakcoder/security/advisories/new>.
3. Include: description, steps to reproduce, potential impact, and any
   suggested fix.
4. You will receive an initial acknowledgement within 72 hours.
5. We aim to release a fix within 14 days of confirmed vulnerability.

## Security Architecture

### Invariants (non-negotiable)

1. **Model-visible means logged.** Anything that reaches a model request is
   reconstructable from the session JSONL via `derive_messages()`.
2. **Append-only sessions.** Never rewrite or delete session entries.
   Branching = new entry with `parent_id`.
3. **Errors are values.** Tools return `is_error` outputs; library code
   never panics on bad input.
4. **Every streaming event carries delta AND snapshot.** Consumers choose
   their abstraction level.
5. **Abort preserves partial output.** Cancellation tokens thread through
   every async call.
6. **Unsafe is denied** workspace-wide except process-group kill.
7. **Provider failures are typed and durable.** Network, overload, parse, and
   abort outcomes remain attached to the run; user aborts never become a new
   provider request.
8. **Secrets never enter git.** API keys live in `.env` (project) or
   the user `.env` at `data_home()/.env` (gitignored).
9. **Model catalogues are discovered, never hardcoded.** The set of models
   a provider offers is a property of the user's key.
10. **Restricted filesystem access is workspace-rooted.** Symlink and
    traversal escapes fail closed.
11. **Permission changes revoke old capability.** A runtime mode change
    cancels in-flight runs.
12. **Secrets are not ambient tool state.** Bash subprocesses
    receive a small operational environment allowlist.
13. **FullAccess is an explicit human trust decision.** Never selected
    automatically after a denial or failure.
14. **Model tools cross a broker boundary.** Built-in tools execute through
    a versioned broker protocol in a disposable process group.
15. **Unattended surfaces fail closed.** Gateway ships disabled; auto-denies
    approval gates on unattended turns.

### Implemented Hardening

- Read/glob/grep paths outside canonical workspace are denied in read-only
  and workspace-write modes; symlink escapes have regression coverage.
- Bash receives an allowlisted operational environment instead of all host
  variables.
- Seatbelt/Landlock deny network in restricted modes and scope writes to
  workspace and explicit temp paths.
- Every built-in model tool crosses a versioned JSON broker protocol into a
  disposable child process group.
- Docker is not a Runtime backend in this release; selecting it fails closed
  with a typed sandbox error rather than executing outside containment.

### Gateway Security

- Gateway is enabled explicitly with `vakcoder serve --gateway`.
- Bearer authentication is required for every API route other than `/health`,
  the static `/admin` assets, and `POST /auth/login`.
- Browser surfaces authenticate via `POST /auth/login`, which sets an
  HttpOnly, SameSite=Strict `vakcoder_session` cookie. No `Secure` flag is set
  because the supported default is loopback; use a TLS-terminating proxy before
  exposing the listener to an untrusted network.
- The `/admin` SPA shell and static assets are auth-exempt but carry no
  data; every `/admin/api/*` route requires the token or cookie.
- Runtime mutations and run outcomes are recorded in the append-only
  `<data_home>/audit/operations.jsonl` stream. The server does not create a
  second security-event store.

## Dependency Policy

- All dependencies are pinned to exact versions in the workspace manifest.
- New dependencies require a one-line justification.
- `cargo audit` runs in CI on every PR.

## Disclosure Timeline

1. Report received -> 72h acknowledgement
2. Confirmed -> 14-day fix target
3. Fix released -> public disclosure
4. CVE requested if applicable
