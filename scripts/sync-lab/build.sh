#!/bin/sh
# Builds the Linux `vak` binaries into the Docker volume the lab mounts.
# Run from the repository root.
set -e
docker run --rm -v "$PWD":/src:ro -v vk-target:/target \
  -v vk-cargo:/usr/local/cargo/registry -w /src -e CARGO_TARGET_DIR=/target \
  rust:1-bookworm cargo build -p vak -p vak-server --bins
