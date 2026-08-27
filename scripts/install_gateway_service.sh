#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
printf '%s\n' 'note: installing the managed Runtime gateway service.' >&2
printf '%s\n' '      Service units always point at the canonical managed runtime-bin path.' >&2

cd "$ROOT_DIR"
cargo build --release -p vakcoder --no-default-features
target/release/vakcoder self install
