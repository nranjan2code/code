# Security policy

## Supported versions

Security fixes target the current `main` branch and the latest release. Check
the workspace version in [`Cargo.toml`](Cargo.toml) and the release record in
[`CHANGELOG.md`](CHANGELOG.md). Versions below the supported 2.0.0 baseline
are not maintained. Backports to older releases are not guaranteed.

## Report a vulnerability

Please use a [private GitHub vulnerability report](https://github.com/nranjan2code/code/security/advisories/new).
Do not put exploit details, credentials, or private data in a public issue.
Include the affected version, steps to reproduce, likely impact, and any
suggested fix. If GitHub private reporting is unavailable to you, contact
[@nranjan2code](https://github.com/nranjan2code) to arrange a private channel
without sending the vulnerability details publicly.

We will review the report, coordinate a fix and disclosure when appropriate,
and communicate through the private report. There is no fixed response or
release-time guarantee.

For ordinary usage bugs and questions, use the
[public issue tracker](https://github.com/nranjan2code/code/issues).

## Security model

The current [agent security design](docs/design/24-agent-security.md) records
the threat model and priority order. [`AGENTS.md`](AGENTS.md) lists the
enforceable invariants, including workspace-rooted restricted access,
permission revocation, brokered tool execution, and recipient-scoped secrets.
These documents are the source of truth for current behavior; older security
summaries can become stale as the runtime changes.
