Status: implemented in 3.0.24

Tavily Integration via MCP
==========================

## Overview

Tavily is a search-as-a-service API that vak integrates with through the
MCP (Model Context Protocol) bridge. The integration is SSRF-guarded: all
HTTP egress flows through the brokered execution model (vak-tools), which
scrubs the subprocess environment and enforces workspace-rooted filesystem
access.

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
