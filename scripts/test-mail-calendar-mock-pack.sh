#!/usr/bin/env bash
# Deterministic, synthetic-only mail/calendar regression pack.
# Provider tests use loopback HTTP/IMAP doubles and temporary vaults; no live
# account, external network, or developer data home is needed.
set -euo pipefail

# This suite builds much of the workspace; debug symbols and incremental state
# are not needed for its pass/fail regression checks and can consume tens of
# gigabytes in a local worktree.
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_TEST_DEBUG=0

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

printf '%s\n' 'Mail/calendar mock regression pack (local builds, synthetic fixtures only)'
npm --prefix crates/vak-client-ui run build:web
npm --prefix crates/vak-client-ui run typecheck:mail-calendar-fixture
cargo test --offline --locked -p vak-mail-calendar
cargo test --offline --locked -p vak-tools mime_parser
cargo test --offline --locked -p vak-core mail_calendar
cargo test --offline --locked -p vak-server --features test-support mail_calendar
cargo test --offline --locked -p vak-server --features test-support --lib restart_requeues_inflight_mail_watch_items_from_the_agent_vault
cargo test --offline --locked -p vak-server --features test-support --test mail_calendar_worker

printf '%s\n' 'Mail/calendar mock regression pack passed.'
