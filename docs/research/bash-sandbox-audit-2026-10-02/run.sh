#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
probe_dir=$(mktemp -d "${TMPDIR:-/tmp}/vak-bash-audit.XXXXXX")
trap 'rm -rf "$probe_dir"' EXIT HUP INT TERM
cp "$repo_root/docs/research/bash-sandbox-audit-2026-10-02/probe.rs" "$probe_dir/main.rs"

cat > "$probe_dir/Cargo.toml" <<EOF
[workspace]

[package]
name = "vak-bash-sandbox-audit-probe"
version = "0.1.0"
edition = "2024"

[[bin]]
path = "main.rs"

[dependencies]
serde_json = "1"
tempfile = "3"
tokio = { version = "1", features = ["full"] }
vak-tools = { path = "$repo_root/crates/vak-tools" }
vak-sandbox = { path = "$repo_root/crates/vak-sandbox" }
vak-permission = { path = "$repo_root/crates/vak-permission" }
EOF

cargo run --offline --manifest-path "$probe_dir/Cargo.toml" \
    --target-dir "$repo_root/target" -- "$@"
