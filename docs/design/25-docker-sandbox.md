# 25 — Docker execution backend

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
3. **Hard caps and privilege reduction**: memory 2g, cpus 2, 256 PIDs, all
   capabilities dropped, `no-new-privileges`, a read-only root filesystem,
   bounded tmpfs, and throwaway containers (`--rm`).
4. **ReadOnly mode** additionally mounts the workspace `:ro`; workspace-write
   mounts it read-write. Both modes use only the bounded writable `/tmp` tmpfs.
5. **Fail closed**: an unreachable daemon surfaces as a failed wrapped
   command (never silently falls back to host execution). A cheap cached
   `docker info` probe powers availability checks.
6. **Two-layer quoting discipline**: BashTool wraps the wrapper in another
   `sh -c`; the inner command is POSIX single-quote escaped here.

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
  can move the complete protocol server into the container once vakcoder ships
  a pinned Linux worker artifact for each supported architecture.
- No `--user` mapping yet: on Linux hosts with plain dockerd, container
  writes are root-owned. macOS/Windows Desktop handle this transparently;
  rootless/docker-userns-remap setups are unaffected.
