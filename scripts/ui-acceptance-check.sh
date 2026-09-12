#!/usr/bin/env bash
# Lightweight, dependency-free acceptance gate for the client presentation
# shell and the admin/navigation surfaces that make it operable.
# Browser suites can use the stable data-testid hooks; this catches accidental
# omission from both source and the generated web bundle in CI/release builds.
set -Eeuo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
UI="$ROOT_DIR/crates/vak-client-ui"
ADMIN="$ROOT_DIR/crates/vak-admin-ui"
fail() { printf '✗ %s\n' "$*" >&2; exit 1; }
require() { rg -q --fixed-strings "$1" "$2" || fail "missing $1 in $2"; }

require 'data-testid="presentation-mode"' "$UI/src/components/WorkspaceHeader.tsx"
require 'data-testid="mode-everyday"' "$UI/src/components/WorkspaceHeader.tsx"
require 'data-testid="mode-advanced"' "$UI/src/components/WorkspaceHeader.tsx"
require 'data-testid="everyday-context-rail"' "$UI/src/components/EverydayContextRail.tsx"
require 'Retry details' "$UI/src/components/EverydayContextRail.tsx"
require 'id="voice-provider"' "$ADMIN/src/App.tsx"
require 'for="voice-provider"' "$ADMIN/src/App.tsx"
require 'id="voice-transcription-model"' "$ADMIN/src/App.tsx"
require 'for="voice-transcription-model"' "$ADMIN/src/App.tsx"
require 'id="voice-synthesis-model"' "$ADMIN/src/App.tsx"
require 'for="voice-synthesis-model"' "$ADMIN/src/App.tsx"
require 'id="voice-realtime-model"' "$ADMIN/src/App.tsx"
require 'for="voice-realtime-model"' "$ADMIN/src/App.tsx"
require 'aria-label="Helpful details"' "$UI/src/components/EverydayContextRail.tsx"
require 'label="Files"' "$UI/src/components/EverydayContextRail.tsx"
require 'label="Next steps"' "$UI/src/components/EverydayContextRail.tsx"
require 'onClick={() => setDockTab(props.tab)}' "$UI/src/components/EverydayContextRail.tsx"
require 'My tasks' "$UI/src/components/Sidebar.tsx"
require 'aria-label={label}' "$UI/src/App.tsx"
require 'aria-label={label} aria-pressed' "$UI/src/App.tsx"
require 'role="complementary"' "$UI/src/App.tsx"
require 'Task details:' "$UI/src/App.tsx"
require 'data-testid="advanced-workspace-dock"' "$UI/src/App.tsx"
require 'aria-label="More workspace views"' "$UI/src/App.tsx"
require 'Everyday' "$UI/src/components/PresentationRenderer.tsx"
require 'Advanced' "$UI/src/components/PresentationRenderer.tsx"
require 'aria-live={pending() ? "assertive" : "polite"}' "$UI/src/components/PresentationRenderer.tsx"
require 'Research a question' "$UI/src/components/ChatPane.tsx"
require 'Write or rewrite' "$UI/src/components/ChatPane.tsx"
require 'Analyze data' "$UI/src/components/ChatPane.tsx"
require 'Plan something' "$UI/src/components/ChatPane.tsx"
require 'composer-lookup-error' "$UI/src/components/Composer.tsx"
require 'intent-hud-error' "$UI/src/components/IntentStrip.tsx"
require '<button type="button" class="cb-copy"' "$UI/src/md.ts"
require 'type="button"' "$UI/src/components/DirectoryPicker.tsx"
require 'sideReconnectTimers' "$UI/src/App.tsx"
require 'Side chat lost its connection' "$UI/src/App.tsx"
require 'es.onerror = () => onError?.()' "$UI/src/api.ts"
require 'Managed work unavailable' "$UI/src/components/WorkModal.tsx"
require 'Could not update managed work' "$UI/src/components/WorkModal.tsx"
require 'Could not send diff comment' "$UI/src/components/DiffPane.tsx"
require 'needs `tabindex="-1"` in markup' "$UI/src/focusTrap.ts"
require 'Remove hook ${index() + 1}' "$UI/src/components/Settings.tsx"
require 'Overview' "$ROOT_DIR/crates/vak-admin-ui/src/App.tsx"
require 'Operate' "$ROOT_DIR/crates/vak-admin-ui/src/App.tsx"
require 'Configure' "$ROOT_DIR/crates/vak-admin-ui/src/App.tsx"
require 'System' "$ROOT_DIR/crates/vak-admin-ui/src/App.tsx"
require 'aria-label="Access token"' "$ROOT_DIR/crates/vak-admin-ui/src/App.tsx"
require '<nav aria-label="Primary">' "$ROOT_DIR/crates/vak-admin-ui/src/App.tsx"
require 'use:trapFocus' "$ROOT_DIR/crates/vak-admin-ui/src/OperationsCenter.tsx"

# The desktop and embedded web hosts ship generated bundles. Keep this gate
# honest about that boundary: a source-only pass must not hide a stale bundle.
if ! rg -q --fixed-strings 'advanced-workspace-dock' "$UI/dist/assets"/*.js; then
  fail 'Tauri bundle is missing the Advanced dock test hook; rebuild crates/vak-client-ui'
fi
if ! rg -q --fixed-strings 'advanced-workspace-dock' "$UI/dist-web/assets"/*.js; then
  fail 'web bundle is missing the Advanced dock test hook; rebuild crates/vak-client-ui'
fi

printf '✓ UI acceptance hooks present (Everyday/Advanced, guidance, rail, renderer, admin IA)\n'
