# Reference UI implementation contract

Status: implementation backlog — not shipped until every acceptance item passes on the installed desktop build.

The supplied Everyday/Advanced reference is the product contract for the desktop chat surface. It is not a marketing mockup. Existing backend capabilities remain available; this work changes presentation and navigation only.

## 1. App shell

- Replace the current developer-first shell with the reference three-column shell: navigation rail, conversation canvas, optional context rail.
- Everyday is the default mode; Advanced is an explicit toggle in the conversation header and composer.
- Preserve native window chrome, keyboard shortcuts, workspace switching, task history, settings, voice, files, skills, approvals, and terminal access.
- Define responsive states for wide desktop, narrow desktop, and context-rail closed.

## 2. Everyday mode

- Human labels: New conversation, Home, My tasks, Reminders, Saved, Recent.
- Conversation header shows title, mode toggle, and a single overflow menu; developer controls remain hidden, not removed.
- User messages, assistant messages, and structured results use the same readable rhythm and card language as the reference.
- Result cards support title, summary, sections, timeline/list content, practical tips, sources, attachments, and next actions.
- Empty states explain what the user can do next; errors use plain language and a retry/repair action.

## 3. Advanced mode

- Retain the existing coding and operations functionality: activity, changes, terminal, tools, model, permission mode, artifacts, and evidence.
- Advanced is a presentation mode, not a separate runtime or permission path.
- Every Everyday result has an equivalent original/technical view.

## 4. Rendering system

- Keep the semantic output contract and adaptive renderer, but add production components for document, itinerary, checklist, comparison, table, chart, recipe, research, code diff, test report, terminal, artifact, approval, and progress states.
- Unknown or invalid structured output must render the original text with a clear recovery affordance.
- No hard-coded domain scenario is allowed in routing; cards are selected from semantic type and validated payload schema.
- Add visual snapshots for each renderer in both modes and at least one unknown-payload fallback.

## 5. Interaction and persistence

- “Use this”, “Keep original”, and “Suggest a change” persist presentation feedback and are visible only where applicable.
- Mode preference persists per user and can be overridden per task without changing runtime authority.
- Voice, attachments, file selection, skills, and send/cancel behavior remain functional in both modes.

## 6. Reliability gate

- Add an end-to-end MCP test using a fake server: list, call, missing action, missing server, malformed arguments, and recovery.
- Validate/normalize compatibility input before schema rejection; never rely on a model obeying `oneOf`.
- Run the test against the installed macOS bundle, not only Rust unit tests.
- Run full clippy, workspace tests, frontend build, desktop build, and a screenshot pass before tagging.

## 7. Release gate

- No `--skip-checks` artifacts are release candidates.
- The installed `/Applications/Vak.app` version must equal the tagged version.
- Docker headless image and macOS bundle must report the same commit and version.
- A release is complete only when the reference checklist, MCP flow, full tests, and clean release script all pass.
