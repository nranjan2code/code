# 21 — Surface boundaries

The TUI and desktop are presentation clients, not alternate agent hosts. They
share the same authenticated Runtime API and IDs as the CLI and admin console.

The terminal process owns stdin/stdout and a disposable render loop. The
desktop process owns its native window and webview. Neither process opens
SQLite, session JSONL, blobs, config files, or provider credentials, and neither
process invokes a tool directly.

When a run is active, clients consume Runtime events and may request
cancellation. Runtime retains partial output and emits the terminal outcome.
Permission, sandbox, capability epoch, and approval state are displayed from
Runtime responses; clients cannot infer or grant them locally.

This boundary keeps every mutation and every model-visible input on one
auditable path while allowing each surface to render for its environment.
