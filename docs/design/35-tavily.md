Tavily Out-of-the-Box MCP Integration
====================================

Status: implemented in 3.0.24

## Overview

Tavily is one peer in vak's curated out-of-the-box MCP integration catalog
(alongside Context7, Exa, Firecrawl, and user-defined servers). It carries no
hardcoded runtime privileges or special architectural status: like any MCP
server, its configuration is inherited across four tiers (Global `~/vak-home` →
Workspace `cwd` → Agent `~/vak-home/agents/<agent_id>` → Session), secrets are
scoped via the credential store (docs/design/44-shared-config.md, "Secrets
Chain"), and its tools (`tavily_search`) are discovered, probed, and
bound dynamically by the Capability Registry. Egress is SSRF-guarded through
sandboxed broker execution.

## Architecture

```
┌──────────────┐    ┌──────────────┐    ┌──────────────┐
│   AGENTS    │    │   vak-core   │    │   MCP Runner │
│   (model)   │───▶│  (prompting) │───▶│  (npx tavily)│
└──────────────┘    └──────────────┘    └──────┬───────┘
                                            │ egress
                                            ▼
                                    ┌──────────────┐
                                    │   Tavily API │
                                    │   (HTTPS)    │
                                    └──────────────┘
```

1. The model proposes a `tavily_search` tool call.
2. `vak-core` resolves the capability through the MCP meta-tool. The `tavily`
   server is discovered from the workspace's MCP manifest (`[mcp.servers]`).
3. The MCP server is spawned as a sandboxed subprocess (`npx tavily-mcp`),
   inheriting only the operational environment allowlist (PATH, HOME, etc.).
4. The server's outbound HTTPS requests are not directly mediated by vak, but
   the MCP server itself is scoped to the `tavily/*` capability namespace.
5. Results are returned through the MCP protocol, wrapped in the model-visible
   session ledger (invariant 1: model-visible means logged).

## Security

- **SSRF guard**: The MCP server subprocess runs with `env_clear` and the
  workspace root as its working directory. It cannot access provider
  credentials or the parent environment.
- **Capability scoping**: `mcp_allow = ["tavily/tavily_search"]` gates which
  tools the model sees. `mcp_network_deny = ["tavily/*"]` can force the server
  offline for compliance.
- **Network deny test**: `crates/vak-core/src/lib.rs` verifies that
  `tavily`'s network access can be forced off via MCP deny rules, while
  `sandboxed` (already denied) stays off.
- **Session logging**: All `tavily_search` results are recorded as
  `ToolResult` entries in the append-only session JSONL. The model never sees
  raw API output without it being reconstructable from the ledger.

## Configuration

```toml
[mcp.servers.tavily]
command = "npx"
args = ["-y", "tavily-mcp"]
trust = "read-only"

[mcp.allow]
patterns = ["tavily/tavily_search"]
```

The `tavily_search` tool is a model-visible capability only when explicitly
allowed. When absent from the allow list, the model receives an
`unknown_capability` error and the request is recorded (not silently
dropped).
