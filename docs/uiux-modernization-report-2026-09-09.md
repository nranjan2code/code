# vak UI/UX modernization report

Status: historical review of the builds named below; current UI behavior
requires a new running-app check.

**Review date:** 2026-09-09  
**Surfaces:** Tauri desktop (`tauri://localhost`), browser client (`/app/`), admin console (`/admin/`)  
**Build reviewed:** initial walkthrough observed local server `3.0.28`; follow-up implementation and verification target repository `v3.0.31` with regenerated desktop/web/admin bundles.
**Method:** read-only product walkthrough, harmless local chat test, desktop and browser screenshots/AX trees, source review, deterministic detector scan, implementation review, TypeScript builds, and Rust compile checks. No settings, approvals, credentials, or external services were changed.

## Executive assessment

vak has a strong foundation: a distinctive warm-dark visual language, a visible workspace boundary, useful run controls, structured outcomes, receipts, and unusually serious permission semantics. The experience still feels like several capable tools placed beside each other rather than one coherent product. The central problem is decision architecture: a person cannot always tell what needs action, which state is authoritative, where a decision will take effect, or what happens next.

The priority is to make one continuous “task operating surface” shared by desktop and web, then let the admin console provide deeper operational views using the same names, status model, and action links. Styling polish should follow that information architecture; otherwise the product becomes attractive while remaining difficult to operate.

## Follow-up implementation ledger

The post-pause implementation pass shipped the following changes across the shared client, desktop host, web host, and admin console:

| Recommendation | Current state | Evidence |
|---|---|---|
| Canonical task state | Shipped | Header projects one status; goal disclosure distinguishes follow-up availability from settled goal records. |
| Approval journey | Shipped | Direct Inbox review route, request locking, full request disclosure, settled feedback, and persistent-rule preview modal. |
| Notification correctness | Shipped | Request-ID dedupe, persisted quiet-hours preference, critical approval bypass. |
| Readiness/provenance | Shipped | Separate model-unavailable wording, runtime version reporting, scope banner, accessible connection live region. |
| Inbox terminology | Shipped | Needs action / Updates grouping and consistent Mark as read language in client and admin. |
| Shell/settings IA | Shipped | Task-only toolbar controls, grouped Settings navigation, explicit Shared/workspace scope. |
| Accessibility/responsive behavior | Shipped | Approval live regions and sticky mobile actions; Workbench tabs and selections expose semantic state. |
| Contextual actions | Shipped | Skills copy a ready-to-use composer command; MCP health text describes automatic first-use behavior; browser terminal denial explains remediation. |

Automated verification passes client/admin TypeScript checks, desktop/web/admin production builds, Rust desktop/server compilation, 191 server unit tests, and whitespace validation. Rendered browser verification also passes for the unauthenticated `/app/` and `/admin/` entry surfaces: both load meaningful Sign in content, show no blank page or framework error overlay, and expose accessible token/sign-in controls. Authenticated task flows still require a valid local access token for deeper interaction coverage.

## Health score

| Nielsen heuristic | Score /4 | Evidence |
|---|---:|---|
| Visibility of system status | 2 | Run state is visible, but desktop/browser showed “Live” and “Connecting…” together; health failures collapse into “Connecting…”. |
| Match to real world | 2 | “Start a task” is approachable; “Ack”, “Dispatch forensics”, “Managed work”, “plan revision”, and scope language need translation. |
| User control and freedom | 3 | Pause, resume, cancel, change plan, archive, and transcript controls exist. Approval decisions remain fragmented. |
| Consistency and standards | 2 | Shared visual tokens, but Inbox/Approvals, Shared/workspace, and result vocabulary differ across surfaces. |
| Error prevention | 2 | Permission explanations are good; persistent approval rules lack a pre-action scope preview. |
| Recognition rather than recall | 2 | Labels and search help, but the icon-heavy toolbar and hidden tools make discovery costly. |
| Flexibility and efficiency | 3 | Shortcuts, split view, side chat, detail levels, workbench, and best-of-N support expert use. |
| Aesthetic and minimalist design | 3 | Calm, focused visual system; too many controls are exposed before a task exists. |
| Error recovery | 2 | Errors are retained, but authentication/health/model failures do not always provide a direct recovery action. |
| Help and documentation | 3 | Inline descriptions and shortcut discovery are good; consequence-specific guidance is missing. |
| **Total** | **24/40** | **Acceptable foundation; significant journey improvements needed.** |

## What is working

- The “Auditor’s Desk” direction is specific and memorable. Near-black surfaces, warm paper text, and sparse terracotta signal feel intentional rather than generic SaaS decoration.
- Workspace, permission mode, model, transcript detail, and sandbox are visible at the moment of delegation. This supports the product promise of inspectability.
- The running task bar is useful: Pause/Resume, Cancel, Change plan, plan revision, and “Paused at safe boundary” give a person real control.
- Structured output rendering is a major advantage. Headings, lists, tables, evidence metadata, and outcome controls are easier to review than a raw model transcript.
- Settings explain the security boundary in plain language, including that approval mode cannot widen the sandbox. This copy should become the pattern used elsewhere.
- Desktop and browser share one client source tree and adapt host capabilities deliberately. That is the right technical basis for a consistent product.

## Priority findings

### P0 — Create one authoritative state model for a task

The live test showed a completed structured response alongside “Ready”, an expanded “Current goal · Active rev 1”, “Result: produced”, “Completion: complete”, “Evidence: none”, “Human review: not_required”, and Accept/Needs work/Reject actions. These may be valid projections of different subsystems, but the UI does not explain the relationship.

Replace this with one task state machine visible in the header and timeline:

`Draft → Working → Needs your decision → Paused → Complete → Accepted / Needs work / Rejected → Interrupted / Failed`.

Every task should have one primary status, one next action, and a secondary “Details” disclosure for goal revision, evidence strength, provider receipt, and review metadata. A completed task must not retain an “Active goal” label without saying “goal remains open for follow-up”.

### P1 — Make approvals a complete, direct decision journey

Inbox approval entries currently route to a read-only transcript (`InboxPage.tsx`), while the actual approval action lives in the chat card. The user arrives because something needs a decision, then has to hunt for the live card.

Use one attention queue with a **Needs action** section. Each approval item must show the task, exact effect, workspace, permission mode, network/sandbox facts, reversibility, age, and buttons for **Allow once**, **Deny**, and **Create rule…**. “Open task” should land on the exact live card; “View transcript” is secondary. Marking read must never resolve an approval.

During the request, disable all decision buttons and show “Allowing…” or “Denying…”. Keep a compact settled receipt in Outcome detail so the user can verify who decided, when, and what changed.

### P1 — Show the complete effect before persistent authorization

`ChatPane.tsx` truncates the prominent target at 160 characters and expanded arguments at 2,000 without an omission notice. A long command can hide its consequential suffix. “Always allow this” also commits a persistent project rule without a pre-action match/scope preview.

Render a full, wrapping, copyable command/request with an explicit “Show all arguments” disclosure. If transport limits exist, state exactly what is omitted and offer a safe download. Before creating a rule, show: exact matcher, workspace/project scope, tools covered, persistence, expiration (if any), and where to revoke it.

### P1 — Separate notification identity from session identity

The five-minute notification dedupe uses a session/source key. A second approval in the same task can therefore suppress the notification for a new gate. Key delivery and dedupe by approval request ID and state; coalesce only repeated delivery of the same unresolved request. The notification should say “2 decisions waiting” when multiple gates exist and deep-link to the first unresolved request.

### P1 — Make Shared vs workspace scope impossible to misunderstand

Settings visually selects Shared while pages describe changes “in this workspace”. Agent settings show saved project configuration while the scope rail says Shared. Permissions are especially sensitive.

Pin an explicit scope banner beside every editable value: **Shared defaults · inherited by 4 workspaces** or **This workspace · `/path`**. Show two values where useful: “Effective: Workspace write” and “Source: Shared”. Use “User” and “Project” consistently in desktop, web, admin, API, and transcripts.

### P1 — Replace readiness ambiguity with a connection state machine

The desktop and web surfaces visibly showed `Live` beside `Connecting…`. The source maps any missing health response to “Connecting…”, even after the event stream is live.

Use separate, explained indicators: **Event stream: Live**, **Model: Ready / Checking / Unavailable**, **Workspace: Ready / Needs trust**, **Server: Offline**. A failed check must provide Retry and Details, not remain in an endless setup state. Announce transitions with an accessible live region.

### P2 — Reduce the new-task toolbar to task-relevant actions

Before a task exists, side question, compare, split view, Inbox, Workbench, Changes, Terminal, and multiple unlabeled icon controls compete with the primary action. Disabled advanced controls increase cognitive load.

New task should contain the composer, workspace, permission mode, model, attachment/skill affordances, and a single **Tools** menu. Reveal task tools after the first message. Keep Inbox in the global navigation and show an unread badge there rather than duplicating it in the task toolbar.

### P2 — Unify Inbox, Approvals, and notifications across surfaces

Desktop calls the area Inbox and uses “Ack all”; admin navigation calls the route Approvals while the page mixes approvals, scheduled results, budget warnings, and heartbeats. Adopt one information model:

- **Needs action:** approvals, failed runs, trust/drift decisions, budget limits.
- **Updates:** completed runs, scheduled results, delivery notices, heartbeats.
- **Read / archived:** retained history.

Use “Mark read” for notices and “Resolve” only for decisions. Every row has a typed event label and a direct route.

### P2 — Make health and admin density serve a clear job

The admin Home page is operationally rich and evidence-backed, but it presents many equal-weight panels: health arcs, queue, live work, pulse, FinOps, sessions, and stream. Keep the density for operators, but lead with three decisions: **Needs you**, **Running now**, **System health**. Every metric should link to the raw evidence or action that changes it. Preserve the excellent “sampled at” and scope context.

### P2 — Fix version and provenance trust signals

Settings hardcodes `Version 0.2.0`, while the running diagnostics report `3.0.28` and the current repository contract is `3.0.31`. Read version from the host and label client/server/build separately when they differ. Include a small “Source: Shared / Project / Session pin” wherever route, model, permission, or capability values are shown.

### P2 — Replace explanatory-only CTAs with contextual actions

“Use in a task” currently only displays a notice that the skill is available from the composer. “Check on next use” explains behavior rather than taking the user there. Either perform the action (open a composer with the skill selected, open the relevant task, start a check) or style the text as an explanation rather than a button.

### P2 — Broaden the first-run mental model beyond coding

The empty state and composer say “build, fix, or explain” and center workspace files. vak is now general-purpose. Add four restrained examples: **Research a question**, **Write or rewrite**, **Analyze data**, **Automate a workflow**. Keep them as one-line inserts into the composer, not a large prompt gallery.

### P2 — Improve browser-specific affordances

The browser correctly omits native dialogs and tray controls, but the empty composer’s AX tree exposed only “text entry area” without the visible task-specific name. Add an explicit accessible label and keyboard instructions. When a browser terminal is disabled, explain “Terminal is disabled by this server” with a link to the server setting rather than silently omitting it.

### P3 — Treat detector findings as consistency/performance cleanup

The deterministic scan found 8 warnings and 0 errors: five side-tab borders and three width/height transitions. The side borders are semantically used for outcome/selection/health states, so they are not automatic defects; preserve the state cue but consider a quieter tokenized accent. Review admin width/height transitions for layout jank and replace with transform/opacity or grid-row animation where measured performance warrants it.

## Recommended target architecture

```text
Global shell
├── Workspaces + task list
├── Attention (Needs action / Updates)
└── Settings (scope pinned: Shared / This workspace)

Task workspace
├── Header: one status + next action + workspace/model provenance
├── Timeline: user request → plan → activity → decision → outcome
├── Decision rail: approval, trust, drift, or budget action when needed
├── Composer: prompt, attachments, skills, permission/model context
└── Evidence drawer: receipts, full arguments, diff, terminal, audit detail

Admin console
└── Same event/status vocabulary, with denser evidence and operational controls
```

The decision rail is the key modernization: it makes human-in-the-loop work visible without turning every transcript into a forensic log. The timeline remains outcome-first; the evidence drawer is available on demand. Desktop and web must use the same semantic event components, while host-specific controls remain explicit.

## Chat event contract for the modern surface

Every event should render with the same anatomy:

1. **What happened:** plain-language title and typed event (`Plan`, `Tool`, `Approval`, `Progress`, `Result`, `Error`, `Human review`).
2. **Why it matters:** one-sentence consequence and affected scope.
3. **Current state:** Working, Waiting for you, Succeeded, Denied, Failed, or Unknown.
4. **What can I do:** one primary action, with safe secondary actions.
5. **Evidence:** receipt, command/request, files, timestamps, provider/model, and audit detail.

Streaming events should show progress without jumping the reader’s scroll position. Reconnection should say whether the transcript is replaying or resyncing. Partial output must remain visible after cancellation. Errors should preserve the original request and offer Retry, Edit and retry, or Leave as-is.

## HIL and notification design

- Approval cards are blocking decision objects, not ordinary messages.
- Notifications distinguish **needs decision**, **completed**, **failed**, **budget**, **security**, and **heartbeat** with text and icon/state, never color alone.
- A notification click always opens the exact object and focuses its primary action.
- Multiple pending actions are counted by unresolved request ID; repeated delivery of one request is rate-limited.
- Native notification, browser notification, tray, and admin Inbox are projections of one durable attention record.
- “Mark read”, “snooze”, “open task”, “approve”, and “deny” are separate verbs with separate confirmation and receipts.
- Quiet hours and per-category notification preferences belong in one place and show the effective scope.

## Accessibility and responsive acceptance criteria

- Every textarea, status, approval, selection, and dynamic error has an explicit accessible name/role/state.
- Approval arrival and settlement are announced; focus moves to the decision only when the user invoked a notification or chose “Review”.
- Selected Workbench tabs expose `aria-selected` or `aria-pressed`, not only CSS classes.
- Keyboard order follows the visual decision order; Esc closes transient panels without discarding drafts.
- At narrow widths, the task toolbar collapses into labeled menus, the composer remains fully usable, and the decision rail becomes a bottom sheet with a persistent action bar.
- Test dark, light, contrast, reduced motion, 200% text size, screen reader, and offline/reconnecting states.

## Phased delivery plan

**Phase 0 — Trust and state correctness (1 sprint):** runtime version/provenance; connection state machine; approval pending lock; request-ID notification dedupe; complete request disclosure; canonical task status projection.

**Phase 1 — Decision journey (1–2 sprints):** unified Needs action/Updates Inbox; direct approval deep links; decision rail; persistent-rule scope preview; settled decision receipts; error recovery actions.

**Phase 2 — Shell and information architecture (1–2 sprints):** task-relevant toolbar; pinned scope banner; consistent Shared/Workspace vocabulary; grouped settings; admin links into the same task objects; general-purpose first-run examples.

**Phase 3 — Finish and scale (1 sprint):** accessibility acceptance matrix; responsive bottom-sheet decisions; motion cleanup; notification preferences/quiet hours; visual regression snapshots for desktop and web.

## Verification backlog

Run these as product scenarios, not only unit tests:

- Two distinct approvals arrive in one task within five minutes; both notifications and both cards remain actionable.
- A 300-character shell command is shown in full before Allow once and Create rule.
- A slow approval response disables the action group and leaves one settled receipt.
- Health endpoint fails while SSE remains live; UI says “Event stream live · Model unavailable” with Retry.
- Shared model and project permission values are edited in separate scopes; effective value and source remain correct after reload and from a second client.
- Browser terminal disabled, server offline, SSE gap/resync, expired login, provider discovery failure, and partial run cancellation each have a visible recovery path.
- Desktop and `/app` show the same task status, approval card, notification category, and outcome semantics at 1440px, 1024px, 768px, and narrow mobile widths.

## Evidence and limitations

Observed directly on the local install: desktop settings, Permissions, Integrations, Skills, Operations, Automations empty state, Inbox empty state, task creation, pause/resume, structured completion, desktop/web new-task screens, and admin Home/Inbox/Setup/Extensions. The harmless chat test requested a local `printf` only; no external transmission or irreversible action was performed. Active approval resolution, screen-reader output, native OS notification delivery, and a true disconnected server were not exercised. The browser overlay path was unavailable because the available CUA interface does not support mutable script injection; detector output is therefore source/CLI evidence rather than a user-visible overlay.

Detector result: 8 warnings, 0 errors across client/admin CSS; five side-tab rules are semantic status/selection accents and three admin layout transitions are P3 performance candidates. Full JSON is retained at `/tmp/vak-ux-audit-20260908/client-detector.json` and `/tmp/vak-ux-audit-20260908/admin-detector.json`.
