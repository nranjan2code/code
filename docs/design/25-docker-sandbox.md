# 25 — Sandbox enforcement

Sandbox selection is part of the immutable Runtime session contract. The
broker applies it immediately before running an effect; no client or model can
select a backend during a run.

## Backends

- Seatbelt confines macOS workers to the registered project and declared
  temporary paths.
- Landlock confines Linux workers when the kernel supports it.
- Docker provides a command-scoped no-network backend with a read-only root,
  resource limits, and the project bind mount.
- `none` is available only when the effective permission policy explicitly
  permits unrestricted execution.

Missing or unverified containment fails closed. The broker never executes a
host worker binary inside a container that does not contain the pinned worker
artifact.

## Configuration

`vak-config` stores the selected backend and image. Runtime records the
effective sandbox in the session contract; changing it advances the capability
epoch and revokes old work and approvals.

## Verification

Tests cover project-root confinement, network denial, read-only enforcement,
resource limits, cancellation, missing daemon behavior, and broker application
of the selected backend. Docker-specific tests run when a Docker daemon is
available; policy and fail-closed behavior are tested without one.
