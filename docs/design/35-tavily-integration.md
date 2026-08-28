# 35 — First-class Tavily integration

Status: **proposed**

## Decision

Tavily should be a first-class integration in the admin console rather than a
copy-pasted generic MCP example. The application owns the Tavily MCP server
definition. An operator only enters a key, chooses whether Tavily is enabled,
and can rotate or revoke the key.

This removes the configuration choices that currently cause failures:

- no user-entered TOML for the command, package, or arguments;
- no user-controlled `network` flag for Tavily;
- no API key in `.vak/config.toml`, session transcripts, logs, or responses;
- no literal `${TAVILY_API_KEY}` reaching a child process when the key is absent;
- no stale running process after an enable, disable, or key rotation.

The generic MCP manager remains available for other servers. Tavily is a
managed adapter with a stable server name (`tavily`) and a fixed set of
capabilities.

## Security and configuration invariants

### Gateway capability overlays

An approved gateway channel may carry a restrictive capability overlay. The
overlay inherits the selected workspace by default and may reduce built-in
tools, MCP server/tool patterns, visible skills, or assigned hooks. An
explicit empty allow list denies that category; a deny pattern always wins.
The overlay is stored with the allowlist entry and included in the CorePool
key, so channels sharing a workspace cannot share a Core with different
capabilities. Enforcement happens before dispatch and again inside the MCP
meta-tool. Secrets remain owned by the workspace integration and are never
stored in or returned to a channel overlay.

1. Every admin mutation requires the existing authenticated admin route and
   is written through the same atomic configuration path as `/config/mcp`.
2. The key is stored only in the canonical user secret file,
   `data_home()/.env`, with restrictive file permissions. It is never stored
   in project `.vak/config.toml`.
3. The managed server always uses the known executable and arguments:
   `npx -y tavily-mcp`. The implementation should resolve the executable to an
   absolute path for launchd/systemd environments, or use the existing
   controlled launcher resolution.
4. The managed server always has outbound network access when enabled. This is
   an internal integration property, not an editable MCP field.
5. Calls still pass through `PermissionEngine`, the brokered MCP registry, and
   the normal approver. The integration does not create a dispatch shortcut.
6. Disable and rotation revoke active Tavily capabilities before the new state
   becomes visible. In-flight calls are cancelled or allowed to finish under
   the existing revocation policy, but no new call may use the old key.
7. A missing, blank, or malformed key produces a reviewable configuration
   error and a disabled integration; it never starts a half-configured server.
8. Health, `/config`, admin snapshots, and audit events report only metadata:
   enabled state, key-present state, last-four fingerprint, route revision,
   and source. They never return the key.

## Admin experience

Add an **Integrations → Tavily** card to the admin console.

### Initial state

Show one of:

- **Not configured** — no key is present.
- **Enabled** — key present, server discovered successfully, and tools are
  available.
- **Configured but disabled** — key is present but Tavily will not be started.
- **Needs attention** — key is present but validation or MCP discovery failed.

The card contains:

- an API-key input with paste support and no value prefilled;
- **Save and enable**;
- **Disable Tavily**;
- **Rotate key** (replace the secret, validate it, then revoke the old one);
- a non-secret status line showing the last validation time and discovered
  tools;
- a link to the audit entry for the last change.

The UI must not render the key after save, put it in a URL, include it in
browser storage, or echo it in an error message. A failed save leaves the
previous working key and state unchanged.

### Safe mutation flow

For save or rotation:

1. Validate input locally for blank/whitespace and an obviously invalid shape.
2. Send the key over the authenticated admin connection.
3. Write the secret atomically to `data_home()/.env` with mode `0600`.
4. Build the managed Tavily server from application constants.
5. Start a sandboxed MCP worker and perform `list` discovery.
6. If discovery succeeds, revoke the old capability set, apply the new
   effective integration state, and emit one audit event.
7. If discovery fails, restore the prior secret/state when a prior working
   configuration exists, otherwise leave Tavily disabled with a diagnostic.

Disable removes the effective server from the registry, revokes active
capabilities, and retains the secret only if the operator chooses **disable**
rather than **remove key**. Provide **Remove key** as a separate destructive
action requiring confirmation.

## Configuration model

Do not represent the managed integration as user-authored
`[mcp.servers.tavily]` TOML. Use a dedicated persisted section containing
non-secret state only, for example:

```toml
[integrations.tavily]
enabled = true
```

The runtime derives the equivalent internal MCP record:

```text
name: tavily
command: <resolved npx path>
args: [-y, tavily-mcp]
env: TAVILY_API_KEY=<resolved from data_home()/.env>
network: true
```

The generic MCP endpoint may display Tavily as managed and read-only, but must
not allow a generic PUT to replace its command, arguments, environment, or
network policy. Existing hand-written Tavily entries should be detected and
shown as a migration warning rather than silently merged with the managed
integration.

## API shape

Add authenticated endpoints under the existing admin/config contract:

| Endpoint | Verb | Purpose |
|---|---|---|
| `/config/integrations/tavily` | GET | Non-secret status and provenance |
| `/config/integrations/tavily` | PUT | Set key and enable, atomically |
| `/config/integrations/tavily/disable` | POST | Disable and revoke capabilities |
| `/config/integrations/tavily/rotate` | POST | Replace key and revalidate |
| `/config/integrations/tavily/key` | DELETE | Remove the stored key and disable |

Responses should include `enabled`, `key_present`, `status`, `tools`,
`last_validated_at`, `revision`, `source`, and a safe `error_code`. They must
not include command-line environment values or raw child-process output when
that output could contain credentials.

## Runtime and service behavior

The managed Tavily definition must be injected into every trusted execution
surface through the same effective MCP table: CLI, TUI, desktop, gateway,
Telegram, flows, plans, evals, and subagents. Untrusted project configuration
must not be able to add, replace, or disable the user-level managed
integration.

When the integration changes, the running Core must:

- invalidate the cached MCP inventory;
- revoke the old Tavily capability set;
- terminate or recycle the old Tavily worker;
- start discovery with the new state on the next eligible turn;
- publish the new revision and provenance to health/admin views.

Service units must continue to use the captured workspace, canonical `HOME`,
and the canonical `data_home()/.env`. A service restart after a change must
produce the same effective Tavily state as the admin process without requiring
manual edits to a plist or systemd unit.

## Validation and diagnostics

`doctor` should add a Tavily check with separate outcomes:

- disabled by operator;
- no key configured;
- key present but `tavily-mcp` launcher unavailable;
- worker starts but MCP handshake/discovery fails;
- configured and healthy.

`doctor --repair` may repair only mechanical issues such as a missing managed
runtime directory or stale generated service state. It must not invent,
rewrite, or guess an API key.

The admin UI should offer **Test connection** as a read-only discovery check.
It should report the failing stage—secret lookup, launcher, worker start,
handshake, or tool discovery—without displaying the secret.

## Migration from manual MCP configuration

On load, detect `[mcp.servers.tavily]` and classify it as legacy. The admin
console should offer:

1. Copy the existing key into the canonical secret store only if a key is
   available through a supported secret source.
2. Replace the legacy server with the managed integration.
3. Validate discovery.
4. Preserve an audit record of the migration.

Never read an API key from an invalid TOML document as a special case. A
malformed config must be reported and repaired through the authenticated
config writer, with the old file backed up according to the normal config
atomicity policy.

## Acceptance tests

- Admin save creates valid TOML and a `0600` secret file; the key is absent
  from config responses, logs, events, and transcripts.
- Restarting gateway, Telegram, desktop, and CLI produces the same managed
  Tavily state.
- Empty or invalid keys do not replace a working key.
- Rotation cannot leave both old and new workers/capabilities active.
- Disable prevents new Tavily calls and revokes active capabilities.
- Removing the key disables Tavily and makes `doctor` report “no key”.
- Tavily discovery succeeds with the fixed command and `network = true`.
- Generic MCP PUT cannot mutate the managed Tavily command or network policy.
- Untrusted project config cannot override the managed integration.
- A legacy hand-written Tavily entry is surfaced as a migration warning.
- Every execution path still records the normal permission decision and audit
  receipt for a Tavily call.

## Implementation order

1. Add the typed `TavilyIntegrationConfig` and secret-store helpers.
2. Add the managed runtime adapter and capability-revocation hook.
3. Add authenticated server endpoints and audit events.
4. Add admin UI card and connection-test states.
5. Add health/doctor output and legacy migration warning.
6. Add cross-surface and service-restart tests before enabling the UI by
   default.
