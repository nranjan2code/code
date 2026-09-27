# 24 — Agent security boundaries

Status: **implemented and hardened in 3.0.8; current in 3.0.10**. Permission lease revocation,
forwarded approval cancellation, pooled permission-ceiling refresh, and
fail-closed budget admission are covered by runtime checks and regression tests.

## Threat model

Treat model output as adversarial even when the user is trusted. Untrusted
repository files, web pages, issue text, MCP results, skills, hooks, memory,
gateway messages, and dependency output can all become indirect prompt
injection. An approval prompt is a user-consent mechanism, not a containment
boundary. A model can misunderstand a command, hide behavior behind an
interpreter or wrapper, or persuade a user to approve an unsafe operation.

The protected assets are host files outside the workspace, credentials,
Vak's policy and session control plane, network identities, external
services, and host availability. Restricted modes must remain safe when the
model deliberately tries to escape them.

**Serving a browser widens the attacker set from "the model" to "anyone who
can reach the port"** (docs/design/48-web-client.md). The default remains
loopback-only, so nothing changes unless an operator opts in; but once it is
opted into, a page the user merely *visits* becomes a participant. Two
attacks that do not exist for a local-only server therefore have standing
defences: DNS rebinding (a visited page resolving its own domain to
127.0.0.1 to become same-origin) and cross-site request forgery (a cookie
the browser attaches to whoever asks). See invariant 34.

## Security failure classes

Security reviews across agent runtimes repeatedly expose the same failure
classes: path-policy composition errors, symlink escapes, trust confusion,
command injection around directory changes, insecure temporary files,
overbroad outbound access, wrapper-based approval bypasses, incomplete
environment sanitization, and missing authorization checks in plugin and
provider paths. A boundary must bind the resolved resource and outbound
destination, not a plausible-looking string. Hardened systems use workspace-only
filesystem tools, default-denied execution, loopback gateway binding, explicit
plugin trust, and deep policy-drift audits.
- Hermes separates credential passthrough by tool/backend, clears the MCP
  environment except for a small safe set, mounts declared credential files
  read-only, redacts common secret forms, supports website restrictions, and
  recommends container or remote execution as the real boundary.
  <https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/security.md>
- OpenHands executes shell and file actions through a runtime inside a Docker
  container instead of giving the model direct host-process authority.
  <https://docs.openhands.dev/openhands/usage/architecture/runtime>
- NVIDIA OpenShell treats filesystem, process, network, credentials, and
  inference as distinct enforcement layers. Undeclared filesystem paths and
  egress are denied; policy changes are generation-bound and old connections
  are closed; credentials are injected at the gateway rather than exposed to
  the agent environment.
  <https://docs.nvidia.com/openshell/security/best-practices>
  <https://docs.nvidia.com/openshell/latest/sandboxes/policies>

## Required invariants

1. Full access is an explicit trust decision, never an automatic recovery path.
2. Restricted file tools resolve inside the canonical workspace; symlink and
   traversal escapes fail closed.
3. A permission change revokes in-flight capability before the UI reports the
   new state. Approval waiters cannot keep an old run alive.
4. Provider, gateway, and connector secrets are not ambient subprocess state.
5. The component requesting a privilege cannot edit the policy that grants it.
6. Network access is destination- and operation-scoped. "Can browse" must not
   imply arbitrary TCP egress.
7. Tool-specific checks and the OS boundary cover the same resource. Shell-only
   sandboxing would be insufficient if read/edit/MCP/browser effects bypassed
   the broker or external-worker boundary.
8. Every decision, policy generation, resolved target, approval, denial, and
   egress attempt is reconstructable from the append-only session/audit record.
9. Unsupported enforcement fails closed for security profiles that promise it.
10. Resource ceilings limit process count, CPU, memory, disk, output, and time.
11. The untrusted worker never receives policy, approval, provider, session,
    or gateway authority. It receives one validated operation and returns one
    bounded result.

## Implemented hardening

- Read/glob/grep paths outside the canonical workspace are denied in read-only
  and workspace-write modes; symlink escapes have regression coverage.
- Bash receives an allowlisted operational environment instead of all host
  variables.
- Server permission changes cancel main and side runs and deny pending
  approvals, preventing stale full-access execution.
- Seatbelt/Landlock deny network in restricted modes and scope writes to the
  workspace and explicit temp paths. One exception, measured 2026-09-24: a
  dev-server preview the user configured runs under
  `Sandbox::listening_variant`, which on Seatbelt adds
  `(allow network-bind (local ip "localhost:*"))` and
  `(allow network-inbound (local ip "localhost:*"))`. Outbound connections
  stay denied (a sandboxed connect to 1.1.1.1 fails). Seatbelt cannot restrict
  the accepted side to loopback: a server bound to every interface answered on
  the LAN address, and a remote-address filter refuses `listen()` altogether.
  Agent `bash` never receives the variant. Landlock returns none, so Linux
  previews stay closed until the rule (handle `ConnectTcp` only) is
  implemented and verified there.
- Every built-in model tool now crosses a versioned JSON broker protocol into
  a disposable child process group. The broker validates permissions and owns
  cancellation; the worker exposes exactly one built-in operation, receives a
  scrubbed environment, emits a bounded response, and cannot mutate policy.
  Missing executables, malformed protocol, unsupported restricted sandboxes,
  and non-zero worker exits fail closed.
- The same path is used by normal turns, explore/task workers, static flows,
  dynamic plans, and CLI evals. Flow Bash no longer bypasses the configured
  tool registry, permission decision/approver, or sandbox.
- MCP servers are external workers with an empty-by-default environment,
  explicit configured variables, process-group teardown, workspace cwd, and
  the same Seatbelt/Landlock policy in restricted modes.
- Seatbelt and Landlock use explicit system/toolchain/workspace/executable/temp
  read allowlists instead of granting file-content reads across the host.
  Seatbelt allows global metadata traversal needed to reach approved roots,
  without granting arbitrary home or mounted-volume contents.
- `task` and `session_search` remain broker-owned structured capabilities.
  Task only creates governed child agents whose effectful tools are brokered;
  session search only queries the append-only session index. Workers do not
  receive the session store or orchestration handles.
- Network exposure is opt-in and refuses to be implicit. The listener binds
  loopback and the `Host` header is pinned to loopback names plus exactly
  what `[server] trusted_hosts` lists — exact names only, since a wildcard
  there is a rebinding hole with extra steps. A non-loopback `bind` with an
  empty list is a startup error, not a warning: it would otherwise boot and
  reject every request with a 421 nobody can diagnose. State-changing
  requests carrying a session cookie must also carry a recognised `Origin`;
  requests with no `Origin` (curl, the CLI, bridges) must carry a bearer
  token instead, so the relaxation is not a bypass. `?token=` is accepted
  only from loopback — it exists for the one in-process client that cannot
  set a header, and anywhere reachable by hostname it is a credential in an
  access log. `[server]` is privileged in full, so an untrusted repository
  cannot choose an interface, relax the host check, or open a shell.
- The web terminal is a real PTY and therefore not permission-gated like
  every other effect the client can reach. It is off by default, and
  loopback-pinned even when enabled, so turning it on for local convenience
  does not silently expose a shell to whatever hostname the server answers.
- The opt-in Docker backend contains Bash with no network, a read-only root in
  read-only mode, a disposable writable root in workspace-write mode, bounded
  tmpfs, CPU/memory/PID ceilings, dropped capabilities, and
  `no-new-privileges`. It is command-scoped until a pinned multi-architecture
  Linux worker artifact can run the complete protocol inside the image.

## Residual risks and roadmap

### P0 — completed local broker boundary

- Keep parity tests proving that every production agent constructor uses
  brokered tools and that worker cancellation kills descendant processes.
- Keep broker-owned capabilities small and typed. A new effectful model tool
  must use the worker protocol or a separately reviewed external-worker
  adapter before it can enter a production tool registry.

### P0 — remaining filesystem and credential hardening

- Make policy/config/credential/session files immutable to the worker. Learned
  grants must be written by the trusted broker after validating an exact
  capability, never by the agent process.
- Add descriptor-relative, no-follow filesystem operations to close
  check-to-use races. Canonicalization alone is not a hostile-concurrency
  primitive.
- Add explicit secret handles and per-tool credential injection. Do not restore
  arbitrary environment passthrough; declare the recipient and lifetime.

### P1 — useful network without arbitrary egress

- Add a brokered web capability with DNS/IP validation, redirect revalidation,
  private/link-local/metadata blocking, domain + port allowlists, response-size
  limits, and separate read/write HTTP methods.
- Route all worker egress through an enforcing proxy or network namespace.
  Seatbelt's all-or-nothing network rule is useful but cannot express a safe
  browser.
- Keep model-provider inference on a separate trusted channel. Network denial
  inside Bash does not prevent data placed in model-visible tool output from
  reaching the configured provider.

### P1 — control-plane and supply-chain safety

- Hash and approve skills/plugins before activation; pin versions; scan archives
  before extraction; never auto-install dependencies from model-controlled
  names or URLs.
- Separate trusted user instructions from untrusted document/tool content in
  the session schema and render their provenance to the model and UI.
- Add a security audit command for gateway exposure, writable control files,
  sandbox availability, broad rules, secret permissions, plugin trust, and
  policy/UI drift.

### P2 — stronger isolation

- Promote the Docker command backend to a pinned full-worker image with
  unprivileged identity and seccomp, and add a VM/remote backend for the
  strongest hostile-code profile. Keep Seatbelt/Landlock as the low-friction
  local backend, not the strongest advertised boundary.
- The broker-owned task-environment lifecycle now has disposable writable
  layers; content-addressed snapshots and explicit install/build receipts are
  still future hardening.
- The capability-checked workspace broker and authenticated server message
  endpoints now provide the policy seam. The remaining transport adapter must
  use a broker-controlled socket, policy-generation revocation, destination
  controls, quotas, and no host-gateway access. Never replace `--network none`
  with a shared Docker bridge as a shortcut.
- Add adversarial evaluations for prompt injection, wrapper chains, symlink and
  rename races, worktrees, malicious archives, localhost/metadata SSRF, DNS
  rebinding, approval replay, policy downgrade races, fork bombs, and output or
  disk exhaustion.
