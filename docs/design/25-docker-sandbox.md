# 25 — Docker execution backend
Status: implemented backend, now owned by the greenfield `vak-sandbox` crate.
This document describes Docker's backend semantics only. Environment plans,
candidate manifests, promotion, and Workbench projection are defined in
`docs/design/54-task-environments-and-promotion.md`; this backend must not grow
those responsibilities.

G3 of the platform plan (`22-gateway.md`): remote/containerized execution so
a hosted core can run agent commands off-host, and local runs get a harder
isolation floor than Seatbelt/Landlock alone.

## Design

The sandbox seam declares whether a backend contains the whole worker process
or one tool command. Seatbelt/Landlock contain the disposable broker worker.
Docker is command-scoped: the trusted broker places the wrapped Docker command
into a Bash worker request, so the host worker remains a small protocol adapter
and the model-controlled shell runs in the container.

```
bash request → broker worker protocol → Sandbox::wrap(cmd)
          → "docker run --rm --network none --memory 2g --cpus 2 \
              -v <ws>:<ws>[:ro] [--tmpfs /tmp] -w <ws> <image> sh -c '<cmd>'"
```

Key choices:

1. **Same-path bind mount.** The workspace is mounted at its real absolute
   path inside the container. File tools (read/write/edit/glob/grep) run on
   the host against `<ws>`; bash runs in the container at `<ws>` — one path,
   two enforcement layers, no translation bugs.
2. **No network** (`--network none`) in restricted modes. Tasks needing the
   internet run under FullAccess, which disables sandboxes entirely — same
   rule as today.
3. **Ephemeral root filesystem**: ReadOnly uses `--read-only`; WorkspaceWrite
   uses Docker's private writable container layer. Since every invocation uses
   `--rm`, package managers and compilers may install arbitrary software inside
   the container, but those writes disappear unless explicitly written to the
   mounted workspace. This never installs software on the host.
4. **Hard caps and privilege reduction**: memory 2g, cpus 2, 256 PIDs, all
   capabilities dropped, `no-new-privileges`, bounded tmpfs, and throwaway
   containers (`--rm`). ReadOnly additionally uses a read-only root filesystem.
5. **ReadOnly mode** additionally mounts the workspace `:ro`; workspace-write
   mounts it read-write. Both modes use only the bounded writable `/tmp` tmpfs.
6. **Fail closed**: an unreachable daemon surfaces as a failed wrapped
   command (never silently falls back to host execution). A cheap cached
   `docker info` probe powers availability checks.
7. **Two-layer quoting discipline**: BashTool wraps the wrapper in another
   `sh -c`; the inner command is POSIX single-quote escaped here.

## Task environments

The agent environment owns a session-scoped container lifecycle. A fresh
container is created for the first turn of a session, reused by later turns,
and destroyed when the session's sandbox handle is released:

```text
create(task, workspace, base-image-digest, limits, network-policy)
  → install/build/execute via validated docker exec
  → snapshot immutable environment or discard
```

The task container may have a writable root layer and an unprivileged identity
inside its user namespace. The broker, not the model, chooses its name,
mounts, image digest, limits, and lifetime. Host credentials, the Docker
socket, Vak control files, and unrelated workspaces are never mounted. A
snapshot is content-addressed by base image, environment manifest, and policy
generation; changing any of those creates a new environment.

The host file tools remain permission-checked against the canonical workspace;
the container is the isolation boundary for arbitrary shell installation and
execution. A package installed in the container is therefore available to
later turns in that session but is not installed on the host.

## Brokered workspace network

Network remains `none` by default. Agents use the host-side typed
`agent_network` tool; the model never receives a bearer token and Docker task
containers do not receive the broker socket. A future task environment may
request a named brokered network, but a Docker bridge alone is not an authorization
boundary. The broker must create a per-workspace identity and enforce explicit
workspace-to-workspace edges approved by both workspace policies,
authenticated short-lived task identities, destination validation, quotas,
append-only connection receipts, and revocation that closes existing
connections. Host gateway, metadata services, Docker sockets, and private
address ranges remain blocked unless separately brokered.

Two agents must communicate through a brokered service or authenticated
workspace network, never by guessing localhost ports or joining a shared
unrestricted Docker network. Provider inference and user credentials remain
outside this network.

## Configuration

```toml
[sandbox]
backend = "docker"        # auto | seatbelt | landlock | docker
image   = "alpine:3.20"   # default alpine:3.20
```

- `[sandbox]` is **privileged**: stripped from untrusted project config —
  image selection is supply-chain power.
- `auto` keeps platform defaults (Seatbelt on macOS, Landlock on Linux).
- Explicit `seatbelt`/`landlock` on a platform that lacks them falls back
  to that platform's real backend rather than weakening to off.

## Verification

`crates/vak-core/tests/docker_sandbox.rs` (skips cleanly when no daemon):

1. wrap() structure: mount, workdir, caps, quoting.
2. Live exec: marker output from inside the container.
3. Mount visibility: container write appears on the host workspace.
4. Network deny: busybox wget fails fast under `--network none`.
5. Read-only enforcement: writes to the mounted tree fail; file intact.
6. Config flow: `sandbox.backend = "docker"` yields a docker-named sandbox
   through `Core::effective_sandbox_name()`.

## Known limits / next steps

- Built-in file tools run in disposable local workers under canonical
  workspace permission checks; Docker currently contains Bash, while
  Seatbelt/Landlock can contain the entire local worker. A future worker image
  can move the complete protocol server into the container once vak ships
  a pinned Linux worker artifact for each supported architecture.
- Task-scoped lifecycle is implemented by `DockerTaskEnvironment` and is used
  for agent execution; its private writable layer is retained until the task
  ends or the owner is dropped.
- The workspace authorization broker, authenticated server endpoints, and
  model-facing `agent_network` tool are implemented. The Docker task still
  stays on `--network none`; cross-workspace communication is host-mediated,
  capability-checked, mutually authorized, bounded, and queued.
- No `--user` mapping yet: on Linux hosts with plain dockerd, container
  writes are root-owned. macOS/Windows Desktop handle this transparently;
  rootless/docker-userns-remap setups are unaffected.
