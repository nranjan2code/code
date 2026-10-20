# Admin style and theme review — 2026-10-20

Status: historical. Acceptance record for the admin UI source and rebuilt bundle;
this review does not claim a release or replacement of the installed binary.

## Environment and scope

The installed service is on `127.0.0.1:8901`, running from `~/vak-home`.
The checkout's admin Vite preview on `127.0.0.1:1421` used
`VAK_ADMIN_API_PROXY=http://127.0.0.1:8901`, so it displayed the real installed
state without copying a data home or replacing the service. Real data was read
through the admin UI and supported HTTP routes. No real approval, erasure,
credential, permission, provider or service setting was submitted by this review.
Theme choices changed only the review browser's local preference.

## Findings corrected

- Undefined color, font and radius names resolved to the existing shared tokens,
  rather than adding an alternate token vocabulary.
- Conversation KPI cards, masthead, filters, action rows, workflow icons, inspector,
  receipt/log views and conditional candidate layouts now have matching styles.
- Shared panel headings, cards, input geometry, labels, nested forms, links and
  status chips use the theme; fixed black fills and resting-card shadows were
  removed. Status choices retain readable foregrounds in light and dark themes.
- The Songbird uses the approved color export on light backgrounds and reverse
  export on dark backgrounds, in both the shell and sign-in.
- SVG spend-chart labels meet the 12px minimum. Narrow tables and diagrams scroll
  within their containers; conversation tab bars, controls and long lineage text
  remain within the content pane.
- `trapFocus` is imported as a runtime binding for Doctor and memory dialogs.
  Doctor closes with Escape. Both dialogs trap keyboard focus and return it to
  their opener when dismissed.
- A lowercase `fullaccess` contract now displays the unsandboxed warning in the
  conversation masthead, workflow and inspector. Memory flags render one
  inheritance badge, rather than treating two function objects as true.
- Voice provider/model discovery uses the aggregate-scope resolver; `global` and
  `all` are not sent as nonexistent Agent IDs.

## Route coverage

Each route below was loaded afresh in Light, Dark, High contrast, Frosted light
and Frosted dark, at **1440 × 900** and **390 × 844**: **510 direct-load checks**.
Every pass recorded the actual resolved theme, route, content width and scroll
width. No page-level horizontal overflow remained. Light desktop also inspected
lower-page sections by scrolling the content pane. Repeated long lists use the
same row component; the review does not claim to read every record in them.
Match system was separately checked to resolve to the emulated light and dark
system choice; it uses those same concrete theme blocks.

| Screen or section | Route |
|---|---|
| Home | `#/overview` |
| Inbox | `#/inbox` |
| Conversations | `#/sessions` |
| Runs | `#/runs` |
| Library | `#/library` |
| Automations | `#/automations` |
| Commitments | `#/commitments` |
| Learned abilities | `#/capabilities` |
| Operations | `#/operations` |
| Data | `#/data` |
| Extensions | `#/integrations` |
| Channels | `#/gateway` |
| Projects | `#/projects` |
| Security status | `#/security` |
| Prompts | `#/prompts` |
| Knowledge | `#/memory` |
| Model & providers | `#/settings` |
| Setup | `#/setup` |
| FinOps | `#/finops` |
| Traces & logs | `#/diagnostics` |
| How the models did | `#/measurements` |
| Runtime & pools | `#/operations/runtime` |
| Incidents | `#/operations/incidents` |
| Integrity | `#/data/integrity` |
| Storage | `#/data/storage` |
| Live work | `#/operations/work` |
| Channels & delivery | `#/operations/channels` |
| Automations | `#/operations/automations` |
| Providers | `#/operations/providers` |
| Sandbox | `#/operations/sandbox` |
| Keys | `#/data/keys` |
| Second copy | `#/data/sync` |
| Plugins | `#/integrations/plugins` |
| Social | `#/integrations/social` |
| Skills 13 | `#/integrations/skills` |
| Hooks 1 | `#/integrations/hooks` |
| Connect | `#/gateway/connect` |
| Credentials | `#/gateway/credentials` |
| Defaults & status | `#/gateway/routing` |
| Sources | `#/sources` |
| Global Search | `#/search` |
| Access rules | `#/settings/permissions` |
| Infrastructure | `#/settings/infrastructure` |
| Preferences | `#/settings/preferences` |
| #/security/execution | `#/security/execution` |
| #/security/rules | `#/security/rules` |
| #/security/approvals | `#/security/approvals` |
| #/security/audit | `#/security/audit` |
| #/commitments/portfolio | `#/commitments/portfolio` |
| #/commitments/intent | `#/commitments/intent` |
| #/commitments/policy | `#/commitments/policy` |

## Element and conditional-state coverage

- A populated conversation: workflow, log, drift, receipts, checkpoints/diffs,
  lineage and lifecycle, on desktop and phone. The installed full-access record
  showed its unsandboxed state after the correction.
- Doctor and Forget memory dialogs: opening, phone layout, Tab/Shift-Tab focus
  containment, Escape/cancel and focus restoration. Forget was not confirmed.
- Sources, Items and Alerts; measurement Waiting/Accepted/Dismissed; spending
  Provider/Model; memory scope and age filters. All six unsaved source kinds
  (RSS, YouTube, Hacker News, Reddit, Lobsters, HTTP) and the unsaved alert editor.
- A populated run and its expanded technical details, decision history, timings
  and linked conversation trail.
- The isolated `tests/visual-states.html` fixture: all eight inbox kinds, including
  the pending approval's buttons and arguments, budget and failed-automation
  notices, skill review; schedule and matcher builders, tool access selection,
  evidence levels, buttons, inputs and disabled states. The fixture blocks every
  network request and contains synthetic data only.
- Bootstrap sign-in in all five themes at both sizes; passkey and recovery
  sign-in states. No real credential or passkey ceremony was used.
- The existing model-settings browser fixture passed **16 checks**, including
  aggregate voice scope, delayed discovery, saved model IDs, backup-model
  confirmation, unavailable discovery and provider-neutral selection.

## Verification and reproducibility

- `npm --prefix crates/vak-admin-ui test`: six passing Node checks covering API
  errors, undefined token references, all literal component-class references
  (with documented structural hooks), typography, focus directive bindings and
  conditional badges.
- `npm --prefix crates/vak-admin-ui run build`: typecheck, Vite bundle and source
  manifest regenerated. Both browser fixture TypeScript entry points also
  typechecked separately.
- Record, documentation-path and version checks passed with this change. This
  frontend review does not claim the full Rust workspace gate or a release.
- Temporary screenshots and detailed JSON observations are retained locally at
  `/tmp/vak-admin-review/`, including `acceptance.json`; they are not committed
  because live screens contain installation data.

To repeat: run the admin preview with the proxy above, inspect the route table,
then open `/admin/tests/visual-states.html?view=inbox&theme=light` or
`?view=controls&theme=dark`. Sign-in uses `?view=login&method=bootstrap&theme=light`.
The model fixture is `/admin/tests/model-settings.html`; its Run regression
checks button reports each check without calling the installed service.

## Limits

This is visual and interaction acceptance, not a promise that every administrative
operation succeeded. Destructive actions, real credential enrollment, service
restarts, connector activation and permission changes were not exercised against
the installed home. Absent live bindings, active workers, holds and erasure
receipts were covered by their source/style contracts, not invented live state.
The installed service still serves its previously embedded UI; shipping the
rebuilt bundle requires the normal release/install workflow.
