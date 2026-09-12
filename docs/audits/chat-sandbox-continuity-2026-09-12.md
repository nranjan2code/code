# Chat and sandbox continuity

Status: implementation and deterministic verification; live provider acceptance pending.

The chat canvas was rendering transcript text while discarding the received
semantic timeline. Completion hydration replaced the conversation with a
skeleton. Live immutable transcript updates also recreated assistant rows.
The client now projects completed turns through the typed timeline, decodes
explicit transport fences during streaming/fallback, retains prior turns, and
keeps existing content visible during hydration. Ordinary JSON code remains
ordinary content. Invalid payloads are not repaired into typed claims.

Everyday and Advanced share message identity and action controls. Advanced no
longer opens a dock when selected or displays developer-only presentation
metadata. The main dock controls name Files, Changes, and Terminal. Artifacts
no longer open the dock without a user action. Resize-aware scroll following
keeps readers at their chosen position and honors reduced motion.

Workbench updates are scoped to the selected task. Late snapshot responses
cannot fill another task's panel. During execution a historical snapshot does
not replace newer live execution state. Artifact inspection works even without
execution telemetry, cancels stale loads, and refreshes revised files. Inline,
expanded, and Workbench HTML previews share authenticated relative-asset loading
with an opaque sandbox and blocked network access. The unsafe standalone HTML
popout was removed. Assets must be beside the preview or in its subdirectories.
A general module bundler, CSS import resolver, and external asset fetching are
not provided by this static preview loader; generated applications should build
self-contained output before preview verification.

Bash now reports the actual working directory and discovered absolute file
paths to the model. Runtime dependency/cache directories are excluded from
artifact discovery. A timeout remains an error even when partial files survive;
the result asks for readback and verification rather than asserting success.
No session ledger is rewritten, and no permission or sandbox boundary is relaxed.

Verification uses the client browser fixture in
crates/vak-client-ui/tests/presentation.tsx, Rust tool tests including a timeout
with a preserved file, TypeScript checks, and rebuilt desktop/web bundles.
This is not a full installed-desktop or live-model acceptance run. The existing
server preview endpoint remains available; the client uses authenticated
self-contained previews consistently across its hosts.
