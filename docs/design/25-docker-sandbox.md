# 25 — Docker execution backend

G3 of the platform plan (`22-gateway.md`): remote/containerized execution so
a hosted core can run agent commands off-host, and local runs get a harder
isolation floor than Seatbelt/Landlock alone.

## Design

The existing sandbox seam (`vak_tools::sandbox::Sandbox` — `name()` +
`wrap(command)`) is command-string level and consulted by BashTool only.
A docker backend therefore implements the same trait in **vak-core** (the
composition layer that already owns backend selection) without touching
tool internals:

```
bash tool → Sandbox::wrap(cmd)
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
3. **Hard caps**: memory 2g, cpus 2; throwaway containers (`--rm`).
4. **ReadOnly mode** mounts the workspace `:ro` and layers a writable
   `/tmp` tmpfs, so compilers/pipes behave while the tree stays intact.
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

- Only bash is containerized (seam limitation); host-side file tools keep
  permission-engine confinement. A full exec-server model (à la OpenClaw's
  experimental Codex sandbox-exec path) would move all tools behind one
  protocol — future work.
- No `--user` mapping yet: on Linux hosts with plain dockerd, container
  writes are root-owned. macOS/Windows Desktop handle this transparently;
  rootless/docker-userns-remap setups are unaffected.
