# 44 — Shared configuration and scoped capabilities

## Contract

Vak resolves configuration from broadest to narrowest:

```text
built-in defaults
  → Shared `vak-home` configuration and capability stores
  → project configuration and project capability stores
  → workspace/session/task/bot/chat/subagent pins
```

`~/vak-home` is the durable Shared baseline. A project starts with an empty
`.vak/config.toml` and inherits that baseline. It can add local values,
replace same-named values, or explicitly stop inheriting a capability
category. It never receives a copied effective snapshot. A scoped pin is not
persisted as a wider default.

The product uses `user` and `project` on the wire. Interfaces label those
scopes **Shared** and **This project**. Every editable surface reports both the
selected layer and the effective result, including provenance and whether a
local override exists. Resetting a project value deletes its local intent and
resumes inheritance.

## Storage

| Concern | Shared | Project |
|---|---|---|
| Settings, MCP definitions, hooks | `~/vak-home/.vak/config.toml` | `<cwd>/.vak/config.toml` |
| Secrets | `~/vak-home/.env` | `<cwd>/.env` |
| Skills | `~/vak-home/.vak/skills` | `<cwd>/.vak/skills` |
| Plugins | `~/vak-home/.vak` plugin registry | `<cwd>/.vak` plugin registry |

Secrets never appear in API responses, config TOML, ledgers, or audit detail.
MCP definitions reference `${ENV_NAME}`. Resolution is per Core and ordered
project secret, user secret, process environment. This per-workspace lookup is
required for a multi-tenant `CorePool`; a process-global override cannot
represent two projects using different values for the same variable.

## Capability inheritance

MCP servers are name-keyed: a project definition replaces the shared server
of the same name. Hooks are additive unless the project disables shared hook
inheritance. Skills and plugins follow the same shared-first discovery with
project shadowing. Each category has an explicit project inheritance switch;
turning it off hides the shared category without deleting shared data.

Channel and agent overlays remain restrictive. They can hide or narrow the
workspace's resolved capabilities but cannot recover a capability the
workspace excluded or gain a wider permission mode.

## Admin and Desktop

Admin and Desktop expose the same persistent scope selector. Changing scope
reloads the selected layer; save actions always target that layer. Effective
values are shown as inherited context and are never resubmitted during a
layer write.

The curated MCP catalog is code-owned metadata for verified, runnable stdio
servers. A catalog entry includes its exact command, arguments, required
secret names, network requirement, and upstream documentation URL. Enabling
an entry persists a real server definition and optional scoped credentials;
there are no simulated health states or placeholder connectors.

Initial catalog:

- Tavily (`npx -y tavily-mcp@latest`, `TAVILY_API_KEY`)
- Exa (`npx -y exa-mcp-server`, `EXA_API_KEY`)
- Context7 (`npx -y @upstash/context7-mcp@latest`, optional
  `CONTEXT7_API_KEY`)
- Firecrawl (`npx -y firecrawl-mcp`, `FIRECRAWL_API_KEY`)

## Fresh-install capability seed

An install into the platform default prefix seeds the Shared capability stores
idempotently. The seed never overwrites an existing skill, plugin, or hook.
It provides six instruction-only skills for both developers and general users
(`getting-started`, `research-and-sources`, `planning-and-organizing`,
`debugging`, `code-review`, and `data-and-spreadsheets`), plus two native
starter plugins (`developer-starter` and `everyday-starter`) containing the
additional `software-development` and `writing-and-editing` skills. The
plugins are enabled because they contain instructions only; they do not add
tools, commands, credentials, or network access. A single `session_start`
automation template is also stored disabled, so installing Vak cannot execute
an operator command without an explicit enable action in the admin UI.

The seed runs against the canonical `~/vak-home` Shared root (or the explicit
`VAK_HOME` root used for an isolated installation), never the current project.
Projects and agents discover these entries through the normal inheritance
chain and can shadow, add, or disable categories at their own scope.

## Verification obligations

- A new and an existing empty project both observe a changed Shared value.
- A project override wins only in that project and reset resumes inheritance.
- Layer GET → edit → layer PUT never copies inherited MCP servers or hooks.
- Project MCP credentials are invisible to a Core for another workspace.
- Disabling category inheritance hides shared entries without deleting them.
- Admin and Desktop exercise identical server scope contracts.
- Each curated entry has a real command and is persisted exactly as advertised.
