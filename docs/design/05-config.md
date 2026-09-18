# 05 — Configuration and scoped capabilities (vak-config)
Status: implemented in 2.0.0

The single configuration contract. Merged from the former doc 44
(`44-global-configuration.md`), which described the same layering from a
second angle; two documents for one contract is how the two drifted.

## Layering

Vak resolves configuration from broadest to narrowest:

```text
built-in defaults
  → Shared layer          ~/vak-home/.vak/config.toml   + Shared secret scope
  → project layer         <cwd>/.vak/config.toml        + project secret scope
  → scoped pins           workspace · session · task · bot · chat · worker
```

`~/vak-home` is the durable Shared baseline — a normal directory a person
can inspect, distinct from the platform data home that holds sessions,
ledgers, and gateway state. A project starts with an empty `.vak/config.toml`
and inherits that baseline. It may add local values, replace same-named
values, or explicitly stop inheriting a capability category. **It never
receives a copied effective snapshot**, and a scoped pin is never persisted
as a wider default.

Environment: `VAK_MODEL`, `VAK_PROVIDER`; CLI flags override everything.
`VAK_HOME` relocates the data home and nests `cache/` and `logs/` under it,
and makes the installation self-contained — the Shared layer then lives
under that root rather than the account's `~/vak-home`.

Profiles are named override blocks selected with `profile = "ci"`, for
dev/CI splits without duplicating files.

The wire vocabulary is `user` and `project`. Interfaces label those layers
**Shared** and **This project**. Every editable surface reports both the
selected layer and the effective result, including provenance and whether a
local override exists. Resetting a project value deletes its local intent
and resumes inheritance.

## Rules

- **Provider and model are one atomic route.** Never persisted, applied, or
  reported independently. Model ids come from live discovery, never from a
  source-code catalogue (`AGENTS.md` invariant 9).
- **Unknown keys warn and are never fatal.** A typo'd key is diffed against
  the schema and surfaced, so it is visible rather than silently dead.
- **A GET that seeds a same-shape PUT reports only the layer that PUT
  writes**, never the merged view (`AGENTS.md` invariant 21).
- **Privileged project keys require trust.** `permission_mode`, `allow`,
  `hooks`, `anthropic_base_url`, `mcp.servers`, and project prompt layers are
  ignored until the workspace is trusted, via `Core::new_with_trust`, CLI
  `--trust`, or a per-directory marker under `<data-home>/trusted/`. The
  project secret scope is likewise loaded only when trusted. Restrictive keys
  (`deny`, `ask`) and project guardrails still apply from an untrusted
  project, because they can only narrow.
- **`[ui]` keys are cosmetic-tier.** Unknown values warn and fall back
  (`composer` → emacs, `theme` → dark unless defined under `[ui.themes]`);
  a non-string entry in a theme colour table warns rather than failing the
  whole config. `osc52` defaults false. Accessibility flags are render-only
  and never alter protocol behavior or session content.

The supported surface is discoverable through `vak config dump`:
`profiles.*`, `[route]`, retry/watchdog/circuit-breaker controls,
`allow`/`ask`/`deny`, `[sandbox]`, `[tools]`, `[ui]`, `[hooks]`,
`[mcp.servers]`, `[finops]`, `[goal]`, `[memory]`, `[learning]`,
`[gateway]`, `[automation]`, `[update]`, `[intent]`, and `[commitment]`
(`docs/design/47-commitment-kernel.md`).

## Storage

| Concern | Shared | Project |
|---|---|---|
| Settings, MCP definitions, hooks | `~/vak-home/.vak/config.toml` | `<cwd>/.vak/config.toml` |
| Secrets | Shared secret scope | project secret scope |
| Skills | `~/vak-home/.vak/skills` | `<cwd>/.vak/skills` |
| Plugins | `~/vak-home/.vak` plugin registry | `<cwd>/.vak` plugin registry |

A secret scope is never a plaintext file — it resolves through
`vak_config::credentials` to an OS-native secret service (macOS Keychain /
Windows Credential Manager / Linux Secret Service) or, when none is
reachable, an AES-256-GCM encrypted-file fallback
(docs/design/44-shared-config.md, "Secrets Chain"). Secrets never appear in
API responses, config TOML, ledgers, or audit detail. MCP definitions
reference `${ENV_NAME}`. Resolution is per Core and
ordered project secret → Shared secret → process environment. That
per-workspace lookup is required for a multi-tenant `CorePool`: a
process-global override cannot represent two projects using different values
for the same variable.

## Capability inheritance

MCP servers are name-keyed — a project definition replaces the Shared server
of the same name. Hooks are additive unless the project disables Shared hook
inheritance. Skills and plugins follow the same Shared-first discovery with
project shadowing. Each category has an explicit project inheritance switch;
turning it off hides the Shared category without deleting Shared data.

Channel and agent overlays remain restrictive. They can hide or narrow the
workspace's resolved capabilities but can never recover a capability the
workspace excluded, nor gain a wider permission mode.

## Persisted runtime preferences and frozen sessions

Authenticated `/config` mutations for provider, model, max turns, permission
mode, and theme persist changed fields atomically to the workspace
`.vak/config.toml` *before* the live Core override is applied. Health,
provider discovery, and admin snapshots expose effective values, provenance,
and the route revision.

New-session admission in other local processes refreshes persisted
provider/model, max turns, theme, MCP, hooks, and permission mode; a
permission change revokes live capabilities before apply. CLI, task,
heartbeat, and worker pins stay intentionally transient. Sessions freeze
their provider/model contract at creation; a gateway binding rotates to a
new frozen session on mismatch while preserving the old append-only ledger.

## The curated integration catalog

Code-owned metadata for verified, runnable stdio MCP servers. An entry
carries its exact command, arguments, required secret names, network
requirement, and upstream documentation URL. **Every entry is a peer**;
none is privileged in ordering, presentation, or capability.

| Entry | Command | Secret |
|---|---|---|
| Context7 | `npx -y @upstash/context7-mcp@latest` | `CONTEXT7_API_KEY` (optional) |
| Exa | `npx -y exa-mcp-server` | `EXA_API_KEY` |
| Firecrawl | `npx -y firecrawl-mcp` | `FIRECRAWL_API_KEY` |
| Tavily | `npx -y tavily-mcp@latest` | `TAVILY_API_KEY` |

The application owns each definition, so an operator only supplies a key and
chooses whether the entry is enabled. That removes the configuration choices
that used to cause failures: no user-entered command, package, or arguments;
no user-controlled `network` flag; no key in `.vak/config.toml`, transcripts,
logs, or responses; no literal `${…_API_KEY}` reaching a child process when
the key is absent; and no stale running process after an enable, disable, or
key rotation.

Enable stores the key in the canonical secret scope for the selected layer, writes
the managed server definition and environment reference, enables outbound
network for that server, and hot-applies the change. Disable removes the
effective server and hot-applies that. The GET exposes only
enabled/key-present/network metadata — the key is never returned to any
surface once set. Calls still go through the normal MCP meta-tool, the
permission engine, the broker boundary, and any channel `mcp_allow`/
`mcp_deny` overlay.

There are no simulated health states and no placeholder connectors
(`AGENTS.md` invariant 27).

## Capability seeds

The Shared capability stores are seeded idempotently, and the seed never
overwrites an existing skill, plugin, or hook. It provides six
instruction-only skills (`getting-started`, `research-and-sources`,
`planning-and-organizing`, `debugging`, `code-review`,
`data-and-spreadsheets`) plus two starter plugins (`developer-starter`,
`everyday-starter`) carrying the `software-development` and
`writing-and-editing` skills. The plugins are enabled because they contain
instructions only — no tools, commands, credentials, or network access. A
single `session_start` automation template is stored **disabled**, so
nothing an installer did can execute an operator command without an explicit
enable.

Seeding is a setup action against the canonical Shared root, never a side
effect of placing binaries, and never the current project
(`docs/design/46-stabilization-install-and-onboarding.md`, D6). Projects and
agents discover seeded entries through the normal inheritance chain and can
shadow, add, or disable categories at their own scope.

## Verification obligations

- A new and an existing empty project both observe a changed Shared value.
- A project override wins only in that project; reset resumes inheritance.
- Layer GET → edit → layer PUT never copies an inherited MCP server or hook.
- Project MCP credentials are invisible to a Core for another workspace.
- Disabling category inheritance hides Shared entries without deleting them.
- Admin, Desktop, and CLI exercise identical scope contracts.
- Each catalog entry has a real command and persists exactly as advertised.
- An untrusted project's privileged keys are demoted; its `deny`/`ask` and
  guardrails still apply.
- `[intent]` is **partly** privileged, which is unusual and deliberate. Most
  of the section can only narrow what a turn may do, and a repository
  choosing to give itself fewer tools is harmless, so it survives untrusted.
  Two keys are execution power and are stripped: `autonomy` (`delegated` and
  `autonomous` suppress approval gates) and `escalate = "cloud"` (spends the
  user's credentials on a classification dispatch before the run they asked
  for). A cloned repository must not grant itself either.
