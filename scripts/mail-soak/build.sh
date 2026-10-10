#!/bin/sh
# Builds the Linux `vak` with the provider stand-in seam
# (vak-server/test-support) into its own Docker volume. Run from the
# repository root. Never ship this build: it sends provider calls wherever
# VAK_TEST_PROVIDER_BASE (a loopback origin) says.
set -e
docker run --rm -v "$PWD":/src:ro -v vk-soak-target:/target \
  -v vk-cargo:/usr/local/cargo/registry -w /src -e CARGO_TARGET_DIR=/target \
  rust:1-bookworm cargo build -p vak -p vak-server --bins --features vak-server/test-support
