#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
WITH_TELEGRAM=false
for arg in "$@"; do
  [[ "$arg" == "--with-telegram" ]] && WITH_TELEGRAM=true
done

printf '%s\n' 'note: this compatibility wrapper now uses the managed base installer.' >&2
printf '%s\n' '      It never points a service at target/release and never writes legacy ~/.vakcoder paths.' >&2

cd "$ROOT_DIR"
cargo build --release -p vakcoder --no-default-features
target/release/vakcoder self install

if [[ "$WITH_TELEGRAM" == true ]]; then
  target/release/vakcoder self services-sync com.vakcoder.telegram
fi
