# Plugin ecosystem compatibility and trust research

Status: primary-source research snapshot, 2026-08-30  
Implementation target: `docs/design/39-plugin-ecosystem.md`

## Executive decision

Vak should implement a native, security-governed plugin runtime and import
other ecosystems through format adapters. It should not execute a competing
client's runtime wholesale and should not scrape proprietary marketplaces.

The shared interoperability floor is now:

1. [Agent Skills](https://agentskills.io/specification) for instruction and resource packages;
2. [Agent Plugins 1.0](https://agent-plugins.org/specification) for portable bundles of Agent Skills and MCP server configuration; and
3. [MCP](https://modelcontextprotocol.io/docs/getting-started/intro) plus the [official MCP Registry API](https://registry.modelcontextprotocol.io/docs) for tool interoperability and server discovery metadata.

Codex, Claude, GitHub Copilot, Cursor, and Gemini add useful client-specific
components around that floor. Vak can safely normalize declarative components
that map to an existing Vak boundary. Client-specific executable semantics are
disabled unless Vak has a dedicated adapter and can inventory and authorize
them before dispatch.

## Compatibility matrix

| Ecosystem | Package/catalog contract | Components | Vak decision |
|---|---|---|---|
| Agent Skills | `SKILL.md`; required `name` and `description`; optional `license`, `compatibility`, metadata, and experimental `allowed-tools` | instructions, scripts, references, assets | Native. Ignore `allowed-tools` as an authorization grant; treat it only as a capability request. |
| Agent Plugins 1.0 | root `plugin.json` with canonical `$schema`; fixed `skills/` and `mcp.json`; reverse-domain client extensions | Agent Skills, stdio/Streamable HTTP/legacy SSE MCP | Native portable import. Validate locally without fetching schemas. Support `PLUGIN_ROOT`/`PLUGIN_DATA`; never expand arbitrary environment variables. |
| OpenAI Codex | `.codex-plugin/plugin.json`; `.agents/plugins/marketplace.json` | skills, MCP/connectors, hooks, apps/assets and client metadata | First-class adapter. Catalog import is metadata only; installation still inspects local bytes and binds consent to a digest. |
| Claude Code | `.claude-plugin/plugin.json`; `.claude-plugin/marketplace.json` | skills, agents, commands, hooks, MCP, LSP, themes/output styles/monitors | First-class adapter. Normalize supported components; retain unsupported ones as visible diagnostics. Support pinned Git/GitHub and local catalog sources without inheriting trust. |
| GitHub Copilot | root or alternate `plugin.json`; `.github/plugin/marketplace.json`, with Claude catalog fallback | skills, agents, commands, hooks, MCP, LSP; Agent Plugins opt-in | First-class adapter. GitHub explicitly accepts Agent Plugins and Claude marketplace locations. Preserve managed-policy precedence as a future organization policy input. |
| Cursor | `.cursor-plugin/plugin.json`; `.cursor-plugin/marketplace.json`; Agent Plugins | skills, rules, agents, commands, hooks, MCP, variables; portable Agent Plugins | First-class adapter for package/catalog metadata. Cursor's public marketplace is a discovery surface, not an undocumented API to scrape. |
| Gemini CLI | root `gemini-extension.json`; GitHub/local install and gallery | context, commands, hooks, skills, subagents, MCP, policies, themes, settings | Dedicated adapter. Imported policy may only narrow or ask; it can never add an allow or bypass Vak permission decisions. Secret settings map to Vak secret handles, never manifest values. |
| MCP Registry | versioned `server.json` and REST/OpenAPI registry | package/remote transport metadata, arguments, environment/secret inputs, repository, version | Registry adapter, not plugin adapter. Validate namespace evidence and artifact hashes, then perform independent scanning and approval. |
| OpenCode | in-process JS/TS or npm module | broad runtime hooks and code transformation | Do not execute as a compatible plugin. Offer a migration report for separately portable Agent Skills/MCP only. |
| VS Code/Open VSX extensions | `package.json`/VSIX; editor extension host | arbitrary extension code and UI | Do not ingest as agent plugins. Reuse supply-chain lessons: signatures, verified publishers, malware/secret scanning, blocklists and enterprise allowlists. |

## Security and provenance findings

Agent Plugins requires package-contained paths and defines narrow failure
boundaries: an invalid skill or MCP entry is skipped without silently changing
the meaning of other valid components. Its remote MCP format forbids embedded
credentials and requires HTTPS away from loopback. These are minimum format
rules, not a sandbox.

The [MCP Registry overview](https://modelcontextprotocol.io/registry/about)
states that namespace authentication establishes publisher accountability, but
the registry delegates actual code scanning to package registries and
downstream aggregators. Therefore a Vak UI must never label an MCP server
"safe" merely because it appears in the official registry. The registry's
[package guidance](https://github.com/modelcontextprotocol/registry/blob/main/docs/modelcontextprotocol-io/package-types.mdx)
also makes client-side SHA-256 validation material for MCPB artifacts.

[Cursor's marketplace security documentation](https://cursor.com/help/security-and-privacy/marketplace-security)
documents manual review, open-source requirements, and continued MCP
allow/block policy. [VS Code's extension security documentation](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security)
adds useful defense-in-depth precedents: malware and secret scanning, dynamic
analysis, verified publishers, signed packages, abnormal-usage monitoring, and
a blocklist. Vak should implement these as separate evidence fields rather than
one misleading trust badge.

## Required trace chain

Every catalog result, install, activation, and dispatch must be explainable by
one immutable chain:

```text
catalog source identity + fetched revision
  -> catalog snapshot digest + parser/format version
  -> entry source locator + declared revision/hash
  -> fetched package digest
  -> normalized manifest digest + capability inventory
  -> install registry generation + operator/license decision
  -> activation generation + permission/policy decision
  -> session-frozen plugin contract
  -> component invocation + tool authorization + result/receipt
```

Audit records are append-only and carry the preceding digest/generation where
applicable. Catalog verification, publisher verification, artifact integrity,
source-code review, malware scan, runtime sandbox, connection health, and
permission approval are distinct facts.

## Legal boundary

Interoperable parsing of an openly documented file format is different from
copying or redistributing the plugins it indexes. Each package's license and
service terms govern installation, modification, mirroring, and redistribution.
A missing license is not permission. Vak may record and link to a package from
a documented source, but its curated marketplace must not mirror package bytes
without an applicable license or publisher agreement.

Marketplace names, publisher logos, and verification marks are source claims,
not Vak endorsements. OAuth credentials and marketplace authentication remain
owned by their respective services. This architecture is a technical and
product rule; release counsel should review the publisher agreement and privacy
terms before Vak operates a public submission marketplace.

## Claim-to-source ledger

| Claim | Primary source | Confidence |
|---|---|---|
| Agent Skills requires YAML frontmatter with `name` and `description` and defines optional resource directories | [Agent Skills specification](https://agentskills.io/specification) | High |
| Agent Plugins 1.0 standardizes only skills and MCP in its portable core and requires a canonical schema identifier | [Agent Plugins specification](https://agent-plugins.org/specification) | High |
| Agent Plugins uses fixed locations and package containment, and credentials are not portable manifest data | [Agent Plugins specification](https://agent-plugins.org/specification) | High |
| Claude supports repository/local/remote marketplaces, pinned sources, versioned caches and client-specific components | [Claude marketplace docs](https://code.claude.com/docs/en/plugin-marketplaces), [Claude plugin reference](https://code.claude.com/docs/en/plugins-reference) | High |
| GitHub Copilot accepts its own manifests, Agent Plugins, and Claude marketplace locations | [Copilot CLI plugin reference](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-plugin-reference) | High |
| Cursor supports Agent Plugins and a richer Cursor-specific format and manually reviews public marketplace submissions | [Cursor plugin docs](https://cursor.com/docs/plugins), [Cursor marketplace security](https://cursor.com/help/security-and-privacy/marketplace-security) | High |
| Gemini extensions package skills, commands, hooks, subagents, MCP, policies and themes; extension policy cannot auto-allow | [Gemini extension reference](https://geminicli.com/docs/extensions/reference/) | High |
| MCP Registry verifies namespaces but delegates code scanning downstream | [MCP Registry overview](https://modelcontextprotocol.io/registry/about) | High |
| VS Code signs and scans marketplace packages and maintains a blocklist | [VS Code extension runtime security](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security) | High |
| OpenCode plugins are executable in-process JS/TS/npm modules | [OpenCode plugin docs](https://opencode.ai/docs/plugins/) | High |

## Research gaps and change controls

- OpenAI's universal directory and Cursor's hosted marketplace are product
  discovery surfaces; no undocumented listing/download endpoint is assumed.
- Agent Plugins and several client plugin systems are evolving. Adapters must
  select semantics from a recognized local schema/version and reject unknown
  executable behavior.
- Marketplace review processes can change without changing package formats.
  Vak records the fetched policy evidence and timestamp instead of hard-coding
  "trusted marketplace" forever.
- Remote MCP OAuth, signatures/transparency logs, and automated malware/dynamic
  scans require their own implementation and verification phases; package
  inspection must not imply those phases already exist.
