# 58 — Admin information architecture and UI system goal

Status: **goal and implementation guidance**. This document is the source of
truth for the next admin-console UX pass. It describes the desired behavior
and structure; it does not change API contracts by itself.

## Objective

Make the admin console easier to navigate, configure, and operate as vak's
capability set grows. Preserve every existing capability and the evidence-first
behavior of the Operations Center while giving each capability one obvious
home, using the right control for each decision, and applying the same layout,
state, accessibility, and interaction rules everywhere.

Primary users are technical operators who need to answer four questions quickly:

1. What needs my attention?
2. What is running, blocked, or failing?
3. What behavior is configured and where did it come from?
4. What evidence supports the current state?

## Non-negotiable product constraints

- Use real API data. Never add sample, mock, placeholder, or invented state.
- This is an implementation goal, not a visual prototype. Every accepted
  screen, control, route, action, and state must be fully wired to the real
  server/API contract, persist through the intended configuration or ledger
  boundary, and be usable end to end.
- Do not leave TODOs, stubs, fake success responses, dead buttons, dead routes,
  hard-coded counts, simulated loading, sample records, “coming soon” panels,
  or disconnected controls in the shipped result. If a capability cannot be
  completed in the current change, do not present a pretend implementation;
  record it explicitly as unsupported and keep the existing working behavior.
- A UI action is complete only when it handles success, validation failure,
  authorization failure, transport failure, cancellation, and stale data; reads
  the resulting state back from the server; and shows the actual outcome to the
  operator.
- A configuration editor is complete only when it reads the correct layer,
  writes the correct layer atomically, preserves fields it does not own, shows
  the effective result and provenance, and does not require a restart unless
  the server contract explicitly requires one.
- A new capability must be reachable from the admin without source edits after
  installation. Discovered capabilities and integrations must be loaded from
  their real registry/API and must report probe failures as state with a reason,
  never as an empty successful list.
- Preserve append-only evidence, permission-before-dispatch, and per-turn route
  receipts. UI improvements must not bypass server authorization.
- Keep Global/User and Workspace/Project configuration semantics explicit;
  never show an effective merged value when the screen is editing one layer.
- Inherited, overridden, disabled, unavailable, unknown, and empty are distinct
  states and must remain distinguishable.
- Secrets are write-only. Show configured/not configured and last-known metadata,
  never the secret itself.
- Existing advanced syntax remains available through an explicit Advanced
  disclosure; it must not be silently discarded by a friendly picker.
- Hash routes remain bookmarkable and Operations query context must survive all
  internal links.

## Target information architecture

Reduce the primary sidebar to these five operator-oriented groups:

```text
Overview
  Home
  Inbox
  Setup

Work
  Sessions
  Commitments

Operate
  Live work
  Approvals
  Automations
  Deliveries
  Incidents
  Sandbox

Configure
  Model & providers
  Permissions & security
  Channels
  Extensions
  Automations
  Prompts
  Knowledge

System
  Health & diagnostics
  FinOps
  Network & distributed bus
  Appearance & accessibility
  Maintenance
```

This is a target ownership model, not a requirement to break all existing
URLs. Existing routes may redirect or render the new owner until the migration
is complete. A capability must have one canonical management surface.

### Ownership rules

- `Configure > Automations` owns definitions: scheduled tasks, hooks, triggers,
  heartbeat, cadence, quiet hours, and retry policy.
- `Operate > Automations` is read-only runtime status: next fire, last run,
  active run, failure, retry, pause, and durable receipt links.
- `Configure > Model & providers` owns provider keys, discovered models, active
  model, route provenance, fallbacks, voice defaults, and route pins.
- `Operate > Providers` becomes provider health and receipt evidence only, if a
  separate view is still needed.
- `Configure > Channels` owns chats, bots, credentials, routing defaults, and
  capability overlays.
- `Operate > Deliveries` owns pending, retrying, dead-lettered, delivered, and
  replayable outbox records.
- `Configure > Permissions & security` owns permission modes, approval policy,
  tool/MCP/skill/plugin access, sandbox and network policy. Security events are
  a tab in the same area.
- `Configure > Knowledge` owns Memory, Feeds, and Search. Feed diagnostics are
  a workbench tab, not a separate top-level concept.

## Standard screen templates

All screens use the same page frame:

```text
breadcrumb
title + one-sentence purpose
scope + effective-value/source + last-updated + refresh + one primary action
local navigation when the area has multiple views
content
```

Use these templates rather than inventing a page-specific arrangement:

### Overview

Attention strip, 3–5 meaningful metrics, evidence cards, and recent activity.
Home is a triage surface. It must link to the exact filtered destination and
must not duplicate full Operations tables.

### Resource list

Toolbar with search/filter/scope/sort/refresh, summary count, table or list,
row action menu, and consistent empty/loading/error/stale states. Sessions,
channels, plugins, skills, sources, tasks, and incidents should share this
structure.

### Detail

Breadcrumb, title/status/actions, key-value facts, primary evidence, related
resources, and audit history. Existing Operations detail pages are the reference
implementation.

### Configuration

Local tabs or section rail; one decision per card; control; effective value;
inheritance/source; validation; consequence; save/reset state. Long Settings
must be split into focused pages.

### Workbench

Input/query panel, structured result panel, and collapsible raw evidence panel.
Use for feed diagnostics, prompt preview, search diagnostics, and sandbox
inspection.

## Control-selection rules

Choose controls based on the decision, not the storage type:

| Decision | Control |
| --- | --- |
| One known value | Select or radio group |
| One value with descriptions | Card list or segmented control |
| Several known values | Checkbox group or multi-select |
| Reversible enable/disable | Switch with text label |
| Inherit/limit/block | Three-state `AccessPicker` |
| Interval | Preset select with Advanced duration option |
| Schedule | `ScheduleBuilder` with raw cron disclosure |
| Matcher/rule | `MatcherBuilder` with raw pattern disclosure |
| Secret | Secret input + configured state + replace/remove |
| Provider/model | Discovered combobox + explicit Custom ID mode |
| URL | URL input + validation/test action |
| Path | Workspace-scoped validated path control |
| Numeric limit | Number input with units, min/max, effective value |
| Long policy text | Textarea + preview + inheritance explanation |
| Runtime evidence | Read-only table, badge, or detail panel |
| Destructive action | Danger button + consequence confirmation |

Never use a free-text field when the valid vocabulary is known. Free text is
appropriate for model IDs, URLs, paths, policy text, patterns, and other values
the server explicitly treats as open syntax. Advanced text must be opt-in and
must preserve values the picker does not understand.

Every editable control must expose its label, current value, validation state,
scope, source, and when the change takes effect. Placeholders are hints, never
the only accessible name.

## Shared UI components

Add or standardize reusable components in `crates/vak-admin-ui/src`:

- `PageHeader`
- `ScopeBadge` and `EffectiveValue`
- `SaveBar`
- `StatusBadge`
- `MetricCard`
- `FilterToolbar`
- `ResourceTable`
- `EmptyState`, `LoadError`, `StaleState`, and `UnavailableState`
- `KeyValueGrid`
- `EvidencePanel`
- `DangerConfirmation`
- `AdvancedDisclosure`
- `InheritanceNotice`
- `ActionMenu`

Shared components must use the existing warm-dark design tokens, status-color
semantics, radius, typography, focus, and reduced-motion rules. Status color is
never the only state channel; pair it with text, shape, or an icon.

## Required configuration coverage audit

Before declaring the redesign complete, compare `vak-config::FileConfig` and
the authenticated admin API with the UI. Each configuration area must be
classified as one of:

1. editable in the canonical UI;
2. observable but intentionally read-only;
3. safely editable only through an advanced control; or
4. explicitly unsupported with a reason shown to the operator.

The audit must explicitly cover route/fallbacks, intent, goals, automation,
heartbeat, update, tools, stop policy, server/network/web terminal, sandbox,
feeds, FinOps, distributed bus, voice, plugins, MCP, skills, hooks, and tasks.

For every API resource, verify loading, empty, error, stale, unauthorized, and
partial-data behavior. An error must never render as an empty configured list.

## Accessibility and responsive requirements

- Every input has a visible label or programmatic accessible name.
- Sidebar, tabs, tables, dialogs, menus, and builders are keyboard usable.
- Focus indicators remain visible in all themes.
- Status is communicated with text or structure as well as color.
- Destructive confirmation explains exact scope and recoverability.
- At narrow widths, sidebar becomes a drawer, header actions wrap, two-column
  layouts collapse, tables become labeled stacked rows, and hover is never
  required for important actions.
- Reduced-motion users do not receive layout-thrashing transitions.

## Delivery order

1. Build the shared page frame, scope/effective-value treatment, state
   components, and control primitives without changing behavior.
2. Regroup navigation and add redirects while retaining existing deep links.
3. Split Settings into focused configuration pages.
4. Consolidate automation ownership and separate definitions from runtime state.
5. Consolidate provider/model, channel, delivery, and security ownership.
6. Migrate lists and details to shared templates.
7. Run the configuration coverage audit and repair missing or misleading UI.
8. Run responsive, keyboard, screen-reader-name, loading/error, and visual
   verification across representative screens.

## Acceptance criteria

- The result is production-integrated code, not a mockup or partial scaffold:
  no TODO, FIXME, stub, placeholder implementation, hard-coded demo data,
  dead control, dead route, or fake success path is introduced in the changed
  admin surface.
- Every changed control has a real read/write or read-only evidence path, and
  every mutation is verified by reading the resulting server state back.
- Every changed route is reachable from the intended navigation and deep-link
  behavior is tested directly.
- Any unavailable backend capability is represented honestly with an explicit
  unsupported/unavailable state and reason; it is never hidden behind an empty
  panel or a successful-looking placeholder.
- Every existing admin capability has one documented canonical owner.
- A first-time operator can locate model, permission, channel, automation,
  session, incident, and delivery tasks without knowing implementation names.
- No known-vocabulary choice is presented as an unlabeled free-text field.
- Every editable setting shows scope, effective value, source, and apply timing.
- Every resource screen has consistent list, empty, loading, error, stale, and
  detail behavior.
- No API failure is presented as “nothing configured.”
- Existing deep links continue to resolve or redirect deterministically.
- Operations workspace/time context survives navigation and refresh.
- `npm run check` and `npm run build` pass for `crates/vak-admin-ui`.
- Browser verification covers Home, Settings, Model & providers, Permissions,
  Channels, Extensions, Automations, Sessions, Operations, Feeds, and at least
  one narrow viewport.
- The final review records any intentionally unsupported configuration instead
  of leaving it undiscoverable.
