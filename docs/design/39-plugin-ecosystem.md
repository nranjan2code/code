# 39 — Plugin ecosystem: compatible packages, governed activation, trusted catalogs

## Decision

Vak plugins are versioned capability packages. A plugin may contribute skills,
commands, MCP connections, hooks, presentation recipes, or static assets, but
installation never grants permission. Components enter the same registries and
the same permission-before-dispatch boundary used by local runs, gateway runs,
flows, plans, tasks, subagents, server runs, and desktop runs.

The package kernel ships before any public marketplace. Local packages and
repository catalogs exercise the complete inspect → install → review → enable
→ update → rollback → uninstall lifecycle first. A marketplace is then only a
discovery and artifact source; it cannot define a second runtime or trust model.

## Product promise

The plugin experience should make common daily work installable without making
the agent opaque:

- personal operations: calendar, mail, notes, files, reminders, travel;
- software work: Git hosting, issues, CI, releases, deployments, incidents;
- knowledge work: documents, spreadsheets, presentations, research, meetings;
- business operations: CRM, support, finance, analytics, project tracking;
- creative work: design files, media generation, publishing, asset management;
- local automation: scripts, devices, feeds, backups, and recurring checks.

Every listing says what the plugin does, who publishes it, where its code or
service lives, what data it can reach, what executes locally, which permissions
it may request, and what changes in an update. “Installed”, “connected”,
“enabled”, and “allowed” are distinct states everywhere.

## Package model

The native manifest is `vak-plugin.json`. It is the normalized internal
contract even when the source package is imported from another ecosystem.

```json
{
  "schema": 1,
  "name": "meeting-follow-up",
  "version": "1.2.0",
  "description": "Turn meeting records into decisions and follow-ups.",
  "license": "Apache-2.0",
  "publisher": { "id": "acme", "name": "Acme", "url": "https://acme.test" },
  "components": {
    "skills": ["skills/meeting-follow-up"],
    "commands": ["commands"],
    "mcp": [".mcp.json"],
    "hooks": ["hooks.json"],
    "presentation": ["presentation"]
  }
}
```

An installed record freezes:

- normalized plugin id and semantic version;
- original format and source locator;
- immutable source revision when available;
- deterministic SHA-256 content digest;
- license and publisher metadata;
- inspected component inventory and requested capabilities;
- user or workspace scope;
- enabled state and approvals bound to that exact digest;
- previous digest/version retained for rollback;
- install, update, enable, disable, and removal audit timestamps.

Package bytes live in a content-addressed directory under the canonical Vak
data home. Registry writes and activation-pointer changes are atomic. Updating
installs a new immutable generation, compares capabilities, and only switches
the active pointer after review. A downgrade is the same operation and is
never implicit.

The immutable provenance chain is catalog identity/revision → catalog snapshot
digest → entry locator/pin → package digest → normalized manifest and capability
inventory → registry generation → activation generation → frozen session
contract → component dispatch and permission receipt. No link is inferred from
a display name.

## Compatibility

Vak owns the runtime contract and uses adapters at ingestion:

| Input | Support | Rule |
|---|---|---|
| Agent Skills directory | native | `SKILL.md` plus scripts/references/assets |
| Agent Plugins 1.0 `plugin.json` | native import | canonical portable skills + `mcp.json`; client extensions remain namespaced |
| `.codex-plugin/plugin.json` | import | normalize supported skills, commands, MCP, hooks, assets |
| `.agents/plugins/marketplace.json` | catalog import | catalog metadata only; packages still inspected locally |
| `.claude-plugin/plugin.json` / marketplace | import | skills, agents, commands, hooks, MCP, LSP and metadata through explicit adapters |
| GitHub Copilot plugin / marketplace | import | root/alternate manifest locations, including Agent Plugins and Claude catalog compatibility |
| `.cursor-plugin/plugin.json` / marketplace | import | supported declarative pieces; Agent Plugins use the portable adapter |
| `gemini-extension.json` | import | skills, commands, hooks, agents, MCP, settings; policies may narrow, never allow |
| MCP | protocol | stdio first; streamable HTTP and OAuth are later governed transports |
| MCP Registry `server.json` | catalog import | namespace evidence and install metadata are recorded, then independently inspected |
| MCP Apps UI | isolated future adapter | never mount arbitrary model-authored HTML in the native AST renderer |

Unknown manifest keys are retained as diagnostics where possible and ignored
with a warning, matching the configuration compatibility rule. Unknown
component types fail closed because ignoring executable behavior could make the
capability review incomplete.

Compatibility does not imply redistribution rights. A catalog entry must carry
license/terms provenance. Packages without redistribution permission may be
referenced as remote services when their terms allow it, but are not mirrored
by Vak.

## Trust model

Trust is evidence, not a boolean attached to a marketplace name.

1. **Built in** — shipped with the signed Vak application and reviewed with it.
2. **Verified publisher** — artifact signature chains to a configured publisher
   identity and the digest is present in a signed catalog snapshot.
3. **Trusted source** — the operator approved a repository/catalog identity;
   every new digest still receives an update review.
4. **Local development** — explicit filesystem source, visibly marked, never
   promoted to verified by proximity.
5. **Unverified** — inspectable but disabled; effectful activation requires an
   explicit decision and may be forbidden by workspace policy.

Catalog signing, artifact hashing, and publisher verification answer different
questions and are displayed separately. A valid signature does not make code
safe. A reviewed catalog does not grant runtime permission. A familiar name is
not identity; publisher id and signing key prevent namespace squatting.

Revocation data can disable future installs and warn about active versions. It
does not silently delete local data. Emergency disable is explicit, audited,
and reversible after the operator understands the consequence.

## Installation boundary

All inputs are hostile. Inspection happens in a staging directory before any
active registry can see the package:

1. resolve the source without running package code;
2. enforce download, file-count, expanded-size, and nesting limits;
3. reject absolute paths, `..`, special files, hard links, and symlinks;
4. validate UTF-8/JSON metadata and semantic versions;
5. locate every declared component beneath the package root;
6. inventory scripts, commands, hooks, MCP processes, network declarations,
   environment names, secret handles, and UI resources;
7. compute a deterministic digest over relative paths and file bytes;
8. compare requested capabilities with any installed generation;
9. present the review; installation remains disabled by default;
10. copy into immutable storage and atomically commit the registry entry.

An installer never runs `npm install`, `pip install`, `cargo build`, shell
scripts, hooks, or manifest-supplied commands. A plugin may declare an external
runtime requirement, but satisfying it is a separate operator action. Model
output cannot select a package URL or approve activation.

## Activation and inheritance

Plugin is not a fourth permission tier. It is provenance attached to a
capability declaration:

```text
installed component
    → workspace permission and trust ceiling
    → bot policy (when inherited)
    → chat policy
    → per-call PermissionEngine decision and approver
    → brokered dispatch
```

- Skills affect visibility and instructions, never authorization.
- Commands expand to ordinary logged user prompts.
- MCP tools retain `plugin_id/server/tool` provenance and use the brokered MCP
  decision path; network and secret access are separately declared.
- Hooks are disabled until separately enabled and evaluated at their existing
  execution boundary.
- Presentation contributions compile into the closed schema-v2 registry or a
  separately isolated standard UI host.
- Disabling a plugin revokes its active components before the UI reports the
  new state. An in-flight effect cannot keep an old activation generation.
- Sessions record plugin id, version, digest, and visible component inventory
  in their frozen contract. Updates never rewrite an existing ledger.

Name collisions are never silently lost. Workspace declarations may shadow
user declarations, but inventory APIs return winners and shadowed entries with
scope and provenance.

## Marketplace sources

Registered catalog snapshots are searchable through `vak plugins catalog-search`
and the secured `/plugins/catalog` API used by admin and desktop settings.
Results retain source, scope, digest, license, and enabled state. Catalog drift
is surfaced and never silently offered for installation.

The first sources are local directories and pinned Git repositories. HTTPS
catalogs and a Vak-curated catalog follow once signing and revocation are live.
OpenAI’s universal Plugins Directory or another proprietary marketplace is not
scraped or mirrored. Vak consumes a catalog only through a documented and
licensed interface.

Source policy supports:

- allow/deny by source and publisher;
- pinned branch, tag, commit, or digest;
- manual, notify-only, or reviewed automatic catalog refresh;
- no automatic plugin activation after refresh;
- organization-owned curated catalogs;
- offline snapshots with verifiable metadata;
- compatibility ranges for Vak and component protocols.

Search and ranking are local and explainable. Paid placement never masquerades
as relevance. Security status, maintenance state, permissions, license, and
last verified version are visible before popularity signals.

## UX across surfaces

Desktop and Admin share one information architecture and vocabulary. Desktop
is the personal installation and use surface; Admin adds fleet/workspace policy,
catalog, audit, and revocation controls. CLI and HTTP expose the same states.

### Primary navigation

“Plugins” becomes a first-class destination with four stable views:

- **Discover** — searchable catalog, categories, compatibility, verified source,
  required connections, and capability summary;
- **Installed** — enabled/disabled/update/attention states with scope and source;
- **Connections** — OAuth/MCP accounts, secret handles, health, and revocation;
- **Sources** — local/repository/curated catalogs, pins, sync state, and trust.

Skills remain a component-level view. A user can understand which plugin owns a
skill, inspect standalone skills, and see shadowing without conflating the two.

### Plugin detail

The detail view is a continuous page, not nested cards or a modal maze:

1. identity, publisher, version, source, verification, license;
2. concise outcome and representative daily workflows;
3. component inventory;
4. data and capability access grouped by read, write, network, secrets, local
   execution, automatic hooks, and UI;
5. version history and update capability diff;
6. activity/audit history;
7. one primary state-aware action: Install, Review update, Enable, or Connect.

Capability review uses plain language first and exact technical details on
expansion. Dangerous differences cannot be collapsed by default. Consent is
bound to a digest, scope, and capability set rather than a publisher forever.

### State vocabulary

- Available: catalog entry only.
- Installed: immutable package is present but contributes nothing.
- Needs review: install/update contains unapproved capabilities.
- Enabled: approved components are registered for the selected scope.
- Connected: required external identity is available.
- Blocked: policy prevents activation or dispatch; the UI names the exact tier.
- Update available: a newer inspected generation exists; current stays active.
- Revoked: source warns against the version; operator action is required.
- Incompatible: host/protocol constraint fails without attempting installation.

Loading uses skeletons; empty states teach how to add a trusted source or build
a local plugin. Installation progress names each deterministic stage. Failures
preserve the current active generation and provide a copyable diagnostic.
Keyboard navigation, screen-reader labels, 4.5:1 text contrast, visible focus,
reduced motion, responsive tables, and non-color status labels are required.

The visual system follows “The Auditor’s Desk”: dense warm-dark tonal layers,
one burnt-terracotta action/selection accent, fixed semantic status colors,
small radii, and shadows only for floating overlays. Motion communicates state
in 120–200ms and never delays task entry.

## APIs and CLI

Initial control-plane contract:

```text
GET    /plugins
POST   /plugins/inspect
POST   /plugins/install
GET    /plugins/{id}
POST   /plugins/{id}/enable
POST   /plugins/{id}/disable
POST   /plugins/{id}/update
POST   /plugins/{id}/rollback
DELETE /plugins/{id}
GET    /plugin-sources
POST   /plugin-sources
POST   /plugin-sources/{id}/sync
DELETE /plugin-sources/{id}
```

CLI parity:

```text
vak plugin inspect <path-or-source>
vak plugin install <path-or-source> [--scope user|workspace]
vak plugin list
vak plugin enable|disable|update|rollback|remove <id>
vak plugin source add|list|sync|remove
```

Mutations are authenticated control-plane operations, never model tools. Server
and UI requests return the committed registry generation so clients cannot
present stale activation as current.

## Delivery phases

| Phase | Scope | Exit criterion |
|---|---|---|
| P0 | manifest adapters, hostile package inspection, deterministic digest, immutable registry, local CLI | a local skills-only package installs disabled and can be listed with full provenance |
| P1 | governed skill/command activation, shadow disclosure, server API, Desktop/Admin Installed views | enable/disable is atomic and every session freezes plugin provenance |
| P2 | MCP and hook components, capability diff approval, update/rollback, connection lifecycle | an update adding network/hook access cannot activate without a new review |
| P3 | Git and signed catalogs, publisher identities, revocation, Discover/Sources UI | catalog compromise cannot change an installed digest or silently activate code |
| P4 | remote MCP/OAuth, isolated MCP Apps UI adapter, organization policies | external connectors remain least-privilege and headless-compatible |
| P5 | curated daily-user collection, quality/security review automation | useful cross-domain catalog with reproducible review evidence and no privileged shortcuts |
| P6 | sandboxed workspace execution runtimes (Python & React), quarantined `.vak/scratch/` execution, network gating, UI preview cards and dual-mode preview dock | live in v3.0.16; zero secret leakage, isolated scratch containment, safe client iframe rendering |

## Sandboxed execution runtimes (Phase 6 / v3.0.16)

Vak workspaces require safe, repeatable execution for modern programming and
frontend development workflows without compromising host security or workspace
purity. Shipped in v3.0.16, execution runtimes are provided as built-in,
governed capability plugins:

1. **`python-sandbox`** — provides the `python_eval` tool for executing Python
   scripts, data analysis routines, and package-backed workflows.
2. **`react-sandbox`** — provides the `react_preview` tool for compiling,
   validating, and rendering self-contained React/Tailwind/Lucide component
   bundles.

### Quarantined scratch execution

Workspaces maintain strict isolation between project source code and intermediate
execution artifacts:

- All generated execution scripts, virtual environments (`venv`), installed
  packages (`site-packages`), compilation caches, and preview HTML bundles are
  quarantined under `<workspace>/.vak/scratch/` (specifically
  `.vak/scratch/python/` and `.vak/scratch/previews/`).
- The `.vak/` directory is gitignored by default; ephemeral execution artifacts
  never pollute the user's project git status or source tree.
- Ephemeral assets are promoted into the project source tree only when the user
  explicitly requests an exported file or artifact.

### Scrubbed operational environment

Runtime processes execute in scrubbed subprocess environments (`env_clear`),
preventing ambient secret leakage:

- Only minimal operational system variables (`PATH`, `HOME`, and virtualenv
  environment variables) are passed to child processes.
- All provider API keys (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`),
  gateway tokens, database credentials, and ambient process environment
  variables are stripped.
- Execution is strictly rooted in the canonical workspace directory; paths
  attempting directory traversal outside the workspace boundary fail closed.

### Network governance and package management

Runtime plugins adhere to the privileged network model established for MCP
servers and broker tools:

- Configured globally or per-workspace via `[plugins]` in `.vak/config.toml`:
  `plugins_enabled` and `plugins_network_deny`. Outbound egress is granted by
  the privileged `[plugins] network_allow = ["python-sandbox", "react-sandbox"]`
  key (deny-by-default when absent) and is refused for an untrusted project
  layer — the trusted user config is the grant site. The desktop Settings
  **Plugins** tab exposes a per-plugin "Allow network" toggle that persists
  this key and applies it from the next turn (no session rotation), and the
  plugin listing reports the effective `network_allowed`/`network_denied`/
  `network_allow` so a UI never edits a merged view.
- The plugin store owns the runtime package lifecycle: a built-in runtime
  tool is admitted only while its owning package is installed *and* enabled
  in the store, so the Settings row's Enable/Disable/Remove buttons change
  the executable surface, not just the listing. A store that was never
  written is vacuous (config alone decides; pre-setup homes and synthetic
  test homes stay deterministic); once any root's registry exists, the
  package's own flag decides, workspace-first then shared, and `[plugins]`
  remains the privileged overlay above.
- Network precedence is a strict lattice, decided once at admission: the
  plugin-level `network_deny` list always beats `network_allow` (entry wins,
  `*` wins); an absent allowlist denies by default; the channel overlay's
  `plugins_network_deny` stacks on top of the resolved result and can only
  take egress away. There is exactly one decision point —
  `built_in_runtime_plugin_tools`/`runtime_network` in `vak-core` — shared by
  the tool admission and the name packet, so the two surfaces can never drift.
  A plugin-contributed MCP server's `network` flag resolves through the same
  `effective_plugins()` read, so the Settings toggle and a channel overlay
  reach a contributed server exactly like they reach its runner tool.
- Inbound channel overlays support `plugins_allow`, `plugins_deny`, and
  `plugins_network_deny`.
- When network access is disallowed (`plugins_network_deny = true` or lack of
  explicit workspace network entitlement):
  - Package installation (`pip install`) fails closed with an informative
    diagnostic.
  - Generated React preview bundles enforce a strict Content Security Policy
    (`connect-src 'none'`) preventing client-side network egress.

### Cross-surface preview presentation

Compiled React previews integrate seamlessly across client surfaces:

- **Delivery projection**: Rendered previews emit a self-declared structured
  envelope (`semantic_type: "react.preview"` inside a `vak` fence) carrying
  `preview_id`, `title`, `artifact_path`, `sandbox`, and `connect_src`. The
  delivery worker parses it into an `OutputTimeline` item and the `ui.preview`
  recipe (`crates/vak-delivery/src/skills.rs`) resolves a preview-first layout
  on desktop with deterministic markdown fallback on terminal/telegram.
- **In-stream Chat Cards**: `UIPreviewCard` mounts in the continuous chat
  timeline with responsive viewport controls (Mobile 375px, Tablet 768px,
  Desktop 1024px, Full width), safe iframe sandboxing (`sandbox="allow-scripts"`),
  and React `ErrorBoundary` containment.
- **Right Bar Preview Dock**: Upgraded to dual-mode operation supporting both
  external dev server URLs (`http://localhost:...`) and live compiled Sandboxed
  React UI previews with real-time reload and error inspection.
- **Python envelope parity**: `python_eval` returns its captured stdout/stderr
  verbatim (no surrounding fence, no wrapper), so a `vak` fence a script prints
  is indistinguishable from one a model emits and is consumed by the same
  name-agnostic parser (`structured_outputs_from_text`). There is deliberately
  no python-specific adapter: every tool result — React runner, python, bash —
  opts into a typed envelope the single way. The legacy chat tool row strips a
  `vak` fence from its raw preview so the same call is not rendered twice
  (JSON here, `UIPreviewCard` above); the pre-fence human summary survives.
- **Runtime admission is one decision**: `built_in_runtime_plugin_tools` in
  `vak-core` plus the `runtime_plugin_allowed`/`runtime_network` predicates
  feed *both* the tool vector and the admitted-name packet, so the two can
  never drift the way separate constructions did (docs/design/41). The two
  string keys there (`python-sandbox`, `react-sandbox`) are the built-in
  runtime plugins shipped with the harness; third-party runtime capabilities
  are classified data in the plugin store, never harness-side tables. Each
  network surface has its own decision point — plugin *runner* egress via
  `runtime_network`, and a plugin-contributed *MCP server's* egress via its
  own server config in `effective_mcp` — and both layer the channel deny over
  the resolved result.
- **Subagent previews stay child-scoped**: a subagent runs as its own
  session, so a `react_preview` it renders lives in the child's ledger and
  scratch, reachable through the child's own presentation surface. The
  `task` tool returns only the child's final reply *text* into the parent, so
  no preview fence crosses the boundary by default. Bubbling a child preview
  into the parent timeline is deliberately not automatic: it would require
  either the child echoing the fence in its final text (which the common
  generic parser would already surface) or a recursive child-ledger
  projection, and neither is compositional with an append-only single
  session. A child that must present a preview says so in its reply, exactly
  as any tool result does.

## Non-goals

- A plugin does not load a native Rust dynamic library into the Vak process.
- A plugin cannot introduce an unregistered schema-v2 AST node.
- A marketplace cannot grant permissions or inject secrets.
- Install does not execute dependency managers or build scripts outside the
  isolated scratch sandbox.
- Compatibility does not claim endorsement by another marketplace owner.
- Public ranking, payments, reviews, and publisher analytics do not precede the
  secure local lifecycle.
