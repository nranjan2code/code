#!/usr/bin/env bash
# Lightweight, dependency-free acceptance gate for the presentation shell.
# Browser suites can use the stable data-testid hooks; this catches accidental
# omission from both source and the generated web bundle in CI/release builds.
set -Eeuo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
UI="$ROOT_DIR/crates/vak-client-ui"
fail() { printf '✗ %s\n' "$*" >&2; exit 1; }
require() { rg -q --fixed-strings "$1" "$2" || fail "missing $1 in $2"; }

require 'data-testid="presentation-mode"' "$UI/src/components/WorkspaceHeader.tsx"
require 'data-testid="mode-everyday"' "$UI/src/components/WorkspaceHeader.tsx"
require 'data-testid="mode-advanced"' "$UI/src/components/WorkspaceHeader.tsx"
require 'data-testid="everyday-context-rail"' "$UI/src/components/EverydayContextRail.tsx"
require 'aria-label="Helpful details"' "$UI/src/components/EverydayContextRail.tsx"
require 'Everyday' "$UI/src/components/PresentationRenderer.tsx"
require 'Advanced' "$UI/src/components/PresentationRenderer.tsx"

printf '✓ presentation shell acceptance hooks present (Everyday/Advanced, rail, renderer)\n'
