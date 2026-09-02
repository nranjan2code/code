# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.2.x   | Yes       |
| < 0.2   | No        |

## Reporting a Vulnerability

If you discover a security vulnerability in vak, please report it
responsibly:

1. **Do not** open a public GitHub issue for security vulnerabilities.
2. Email the maintainers at the address listed in `Cargo.toml` or open a
   **private** security advisory at
   <https://github.com/vak/vak/security/advisories/new>.
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
7. **Transient provider failures retry within the frozen route ladder.**
   Retries honor `Retry-After`; never retry user aborts.
8. **Secrets never enter git.** API keys live in `.env` (project) or
   the canonical user `.env` at `~/vak-home/.env` (gitignored).
9. **Model catalogues are discovered, never hardcoded.** The set of models
   a provider offers is a property of the user's key.
10. **Restricted filesystem access is workspace-rooted.** Symlink and
    traversal escapes fail closed.
11. **Permission changes revoke old capability.** A runtime mode change
    cancels in-flight runs.
12. **Secrets are not ambient tool state.** Bash and MCP subprocesses
    receive a small operational environment allowlist.
13. **FullAccess is an explicit human trust decision.** Never selected
    automatically after a denial or retry.
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
- MCP servers are external workers with empty-by-default environment.
- Docker backend: no network, read-only root, bounded tmpfs, CPU/memory/PID
  ceilings, dropped capabilities, `no-new-privileges`.

### Rate Limiting

HTTP inbound requests are rate-limited per source IP:

| Endpoint                    | Default Limit |
|-----------------------------|---------------|
| `POST /gateway/inbound`     | 30 req/min    |
| `POST /sessions`            | 5 req/min     |
| `POST /sessions/{id}/run`   | 10 req/min    |
| `POST /admin/login`         | 20 req/min    |
| All other POST endpoints    | 20 req/min    |

Limits are configurable via `[gateway.rate_limit]` in TOML config.
Exceeded limits return `429 Too Many Requests` with `Retry-After`.

### Gateway Security

- Gateway ships disabled; enable via `[gateway] enabled = true` (trusted
  config) or `serve --gateway` (CLI override).
- `chat_allowlist` restricts which `surface:chat` pairs can send inbound
  messages. An empty allowlist **fails closed**: an unknown chat lands as a
  reviewable `pending` entry (not a flat rejection) unless
  `chat_allowlist_open = true` is set explicitly; a `denied` entry is sticky
  and never re-prompts.
- Approval gates auto-deny on unattended turns unless `[gateway] approvals`
  is set to `"forward"` with a configured approver surface.
- Constant-time token comparison prevents timing attacks on bearer tokens.
- Token values are masked in stderr (only first/last 4 characters shown).
- Browser surfaces authenticate once via `POST /admin/login`, which sets an
  HttpOnly, SameSite=Strict session cookie; the same constant-time check
  applies. No `Secure` flag by design (loopback/LAN-first server); do not
  expose the port to untrusted networks without a TLS-terminating proxy.
- The `/admin` SPA shell and static assets are auth-exempt but carry no
  data; every `/admin/api/*` route requires the token or cookie.
- Mutating admin operations are POST-only so crawlers/prefetchers cannot
  trigger them via GET.

### Security Events

Security-relevant events are logged to `<home>/security-events.jsonl`:

- Auth failures (wrong/missing bearer token, failed logins)
- Rate limit triggers
- Chat allowlist rejections
- Config changes (permission mode, provider keys, MCP servers, hooks)
- FullAccess grants/revocations

The log is append-only, structured JSONL, and lives alongside session data.
The admin console surfaces it live (`GET /admin/api/security`) and the
event hub pushes auth-failure/rate-limit alerts to connected browsers.

## Dependency Policy

- All dependencies are pinned to exact versions in the workspace manifest.
- New dependencies require a one-line justification.
- `cargo audit` runs in CI on every PR.

## Disclosure Timeline

1. Report received -> 72h acknowledgement
2. Confirmed -> 14-day fix target
3. Fix released -> public disclosure
4. CVE requested if applicable
