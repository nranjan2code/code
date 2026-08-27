# 35 — Install and release verification

This is the executable release checklist for the greenfield Runtime contract.
There is one state owner (`vak-runtime`), one installed base, and one client
protocol (`vak-server`/`vak-client`). A clean installation begins with an
empty data home. No installer or service command searches for or converts prior
program files, state directories, or protocols.

## Package boundary

| Package | Owns | Must not own |
|---|---|---|
| `vakcoder` | Runtime process, CLI, gateway, embedded admin assets | a second state store or UI agent loop |
| `vak-tui` | terminal presentation and user input | sessions, config, credentials, or effects |
| `vak-desktop` | native shell and desktop presentation | an embedded Runtime or shell executor |
| external channel adapters | inbound/outbound transport | session state or approval policy |

Every client authenticates, performs the `/version` handshake, and sends typed
commands through the Runtime API. The Runtime is the only component allowed to
open state, acquire workspace leases, authorize effects, or mutate records.

## Clean-install verification

Run the release gate in a disposable data home:

```bash
scripts/build-install.sh --no-service --gates
scripts/release_install_smoke.sh target/release/vakcoder
VAKCODER_HOME="$TMPDIR/vakcoder-clean" vakcoder doctor
```

`doctor` diagnoses the install without assuming any of it works: it reports
build, data home, manifest, launcher, gateway token, provider credential,
service units, receipt, and the authenticated handshake, and names the one
command that fixes each finding. It exits non-zero on any failing check, so it
is usable as a gate.

Then verify, with one gateway process:

1. register a canonical project and create a session;
2. submit a deterministic run and observe delta plus snapshot events;
3. cancel a live run and confirm the partial output and terminal record;
4. read the same session from CLI, TUI, desktop, and admin;
5. create, resolve, and reject an approval through the authenticated API;
6. create a checkpoint, export a backup, restore it into an empty root, and
   verify content-addressed blobs and append-only ledgers;
7. change permission mode and confirm old runs and approvals are revoked;
8. stop the gateway and confirm the runtime receipt and lock are cleaned up.

## Filesystem assertions

The only state paths allowed in a release run are the `vak-config` paths:

```text
<data_home>/config.toml
<data_home>/.env
<data_home>/state.db
<data_home>/sessions/<project>/<session>.jsonl
<data_home>/blobs/<sha256>
<data_home>/audit/operations.jsonl
<data_home>/runtime/gateway.json
<data_home>/locks/runtime.lock
<data_home>/logs/
```

The cache is disposable. A service unit must point at
`<data_home>/runtime-bin/current/bin/vakcoder`; it must never reference a
Cargo target directory, a source checkout, or a client executable.

## Client verification

- TUI uses only `vak-client`, renders server events, and reports API errors.
- Desktop uses only the secured Runtime router and preserves the same IDs,
  status transitions, transcript, memory, tasks, approvals, and diagnostics.
- Admin uses the embedded Runtime router and the same API as other clients.
- Delivery adapters use ordered packets; they never mutate a session or bypass
  approval policy.

## Required CI checks

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
(cd crates/vak-admin-ui && npm ci && npm run build)
(cd crates/vak-desktop/ui && npm ci && npm run build)
scripts/check-release-version.sh
```

The release is accepted only when all checks pass and an artifact-level smoke
run proves that every surface talks to the same Runtime data and no service
starts an additional authority.
