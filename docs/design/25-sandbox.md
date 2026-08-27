# 25 — Sandbox enforcement

Sandbox selection is part of the immutable Runtime session contract. The
broker applies it immediately before running an effect; no client or model can
select a backend during a run.

## Backends

- Seatbelt confines macOS workers to the registered project and declared
  temporary paths.
- Landlock confines Linux workers when the kernel supports it.
- Docker is intentionally unavailable in this build. A Docker selection is
  rejected by Runtime with a typed fail-closed error; it never runs a host
  worker without containment.
- `none` is available only when the effective permission policy explicitly
  permits unrestricted execution.

Missing or unverified containment fails closed. The broker never executes an
effect outside a verified Seatbelt or Landlock worker boundary.

## Configuration

`vak-config` stores the selected backend. Runtime records the effective sandbox
in the session contract; changing it advances the capability epoch and revokes
old work and approvals. The supported values are `seatbelt`, `landlock`, and
`none` (the latter only with explicit unrestricted permission).

## Verification

Tests cover project-root confinement, network denial, read-only enforcement,
cancellation, missing-backend behavior, and broker application of the selected
backend. An unavailable backend is tested for fail-closed behavior without
requiring external daemons.
