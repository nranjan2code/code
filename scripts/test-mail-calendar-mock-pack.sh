#!/usr/bin/env bash
# Deterministic, synthetic-only mail/calendar regression pack.
# Provider tests use loopback HTTP/IMAP doubles and temporary vaults; no live
# account, external network, or developer data home is needed.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

printf '%s\n' 'Mail/calendar mock regression pack (local builds, synthetic fixtures only)'
npm --prefix crates/vak-client-ui run build:web
cargo test --offline --locked -p vak-mail-calendar
cargo test --offline --locked -p vak-core mail_calendar
cargo test --offline --locked -p vak-server mail_calendar
cargo test --offline --locked -p vak-server --lib restart_requeues_inflight_mail_watch_items_from_the_agent_vault
cargo test --offline --locked -p vak-server --test mail_calendar_worker

printf '%s\n' 'Mail/calendar mock regression pack passed.'
