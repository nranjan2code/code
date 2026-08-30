"""
feed_mcp.py — MCP stdio server for the feed pipeline.

Exposes feed search, latest, stats, sources, and item tools
to LLM agents via the Model Context Protocol.
"""

from __future__ import annotations

import json
import logging
import sys
import traceback
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).parent))

from feed_security import sanitize_for_llm, sanitize_mcp_response
from feed_search import get_latest, search
from feed_utils import get_db, get_item, get_stats, init_feed_system

logging.basicConfig(
    level=logging.WARNING,
    format="%(message)s",
    stream=sys.stderr,
)
logger = logging.getLogger("feed_mcp")

PROTOCOL_VERSION = "2025-06-18"

TOOLS = [
    {
        "name": "feed_search",
        "description": "Search across ingested feed items. Returns Tavily-compatible structured results with relevance scores, evidence snippets, and follow-up questions. Use this to find information from RSS feeds, YouTube channels, Hacker News, Reddit, and custom data sources.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query — what you're looking for"
                },
                "tags": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Filter by tags (optional)"
                },
                "since": {
                    "type": "string",
                    "description": "Only return items after this ISO date (optional, e.g. '2026-08-01')"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results (default 10, max 50)",
                    "default": 10
                },
                "source": {
                    "type": "string",
                    "description": "Filter by source name (optional)"
                },
                "include_answer": {
                    "type": "string",
                    "enum": ["false", "extractive"],
                    "description": "Generate an answer from results. 'extractive' picks the most relevant passage.",
                    "default": "false"
                },
                "include_raw_content": {
                    "type": "boolean",
                    "description": "Include full item content in results (default false)",
                    "default": False
                }
            },
            "required": ["query"]
        }
    },
    {
        "name": "feed_latest",
        "description": "Get the latest feed items, optionally filtered by source or tags. Use this to see what's new in your feeds.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "source": {
                    "type": "string",
                    "description": "Filter by source name (optional)"
                },
                "tags": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Filter by tags (optional)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum items to return (default 20)",
                    "default": 20
                }
            }
        }
    },
    {
        "name": "feed_stats",
        "description": "Get feed ingestion statistics: total items, active sources, last ingestion time, items today, and per-source breakdown.",
        "inputSchema": {
            "type": "object",
            "properties": {}
        }
    },
    {
        "name": "feed_sources",
        "description": "List all configured feed sources with their type, trust level, and status.",
        "inputSchema": {
            "type": "object",
            "properties": {}
        }
    },
    {
        "name": "feed_item",
        "description": "Get a single feed item by ID with full content and metadata.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": {
                    "type": "integer",
                    "description": "The item ID"
                }
            },
            "required": ["id"]
        }
    },
    {
        "name": "feed_alerts",
        "description": "List all configured alert rules with their match criteria, delivery targets, and enabled status.",
        "inputSchema": {
            "type": "object",
            "properties": {}
        }
    }
]


def handle_request(request: dict) -> dict:
    method = request.get("method", "")
    req_id = request.get("id")
    params = request.get("params", {})

    if method == "initialize":
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "vak-feeds",
                    "version": "1.0.0"
                }
            }
        }

    if method == "notifications/initialized":
        return None

    if method == "tools/list":
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {"tools": TOOLS}
        }

    if method == "tools/call":
        return handle_tool_call(req_id, params)

    if method == "ping":
        return {"jsonrpc": "2.0", "id": req_id, "result": {}}

    return {
        "jsonrpc": "2.0",
        "id": req_id,
        "error": {
            "code": -32601,
            "message": f"Method not found: {method}"
        }
    }


def handle_tool_call(req_id: Any, params: dict) -> dict:
    tool_name = params.get("name", "")
    arguments = params.get("arguments", {})

    try:
        if tool_name == "feed_search":
            result = handle_feed_search(arguments)
        elif tool_name == "feed_latest":
            result = handle_feed_latest(arguments)
        elif tool_name == "feed_stats":
            result = handle_feed_stats(arguments)
        elif tool_name == "feed_sources":
            result = handle_feed_sources(arguments)
        elif tool_name == "feed_item":
            result = handle_feed_item(arguments)
        elif tool_name == "feed_alerts":
            result = handle_feed_alerts(arguments)
        else:
            return {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "content": [{"type": "text", "text": f"Unknown tool: {tool_name}"}],
                    "isError": True
                }
            }

        sanitized = sanitize_mcp_response(result)
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "content": [{"type": "text", "text": json.dumps(sanitized, indent=2)}],
                "structuredContent": sanitized
            }
        }

    except Exception as e:
        logger.error("Tool error %s: %s", tool_name, traceback.format_exc())
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "content": [{"type": "text", "text": f"Error: {str(e)}"}],
                "isError": True
            }
        }


def handle_feed_search(args: dict) -> dict:
    query = sanitize_for_llm(args.get("query", ""), max_length=500)
    if not query:
        return {"query": "", "answer": None, "follow_up_questions": [], "results": [], "meta": {"total_results": 0, "returned_results": 0, "search_time_ms": 0, "sources_searched": [], "deduplicated": 0}}

    return search(
        query=query,
        tags=args.get("tags"),
        since=args.get("since"),
        limit=min(args.get("limit", 10), 50),
        source=args.get("source", ""),
        include_answer=args.get("include_answer", "false"),
        include_raw_content=args.get("include_raw_content", False),
    )


def handle_feed_latest(args: dict) -> dict:
    return get_latest(
        source=args.get("source", ""),
        tags=args.get("tags"),
        limit=min(args.get("limit", 20), 100),
    )


def handle_feed_stats(args: dict) -> dict:
    return get_stats()


def handle_feed_sources(args: dict) -> dict:
    con = get_db(read_only=True)
    rows = con.execute(
        "SELECT id, name, source_type, url, trust, enabled, check_interval "
        "FROM feeds WHERE removed_at IS NULL ORDER BY name"
    ).fetchall()
    con.close()

    sources = []
    for r in rows:
        sources.append({
            "id": r[0], "name": r[1], "source_type": r[2], "url": r[3],
            "trust": r[4], "enabled": r[5], "check_interval": r[6],
        })

    return {"sources": sources, "total": len(sources)}


def handle_feed_item(args: dict) -> dict:
    item_id = args.get("id")
    if not item_id:
        return {"error": "Missing item id"}

    item = get_item(int(item_id))
    if not item:
        return {"error": f"Item {item_id} not found"}

    return {"item": item}


def handle_feed_alerts(args: dict) -> dict:
    con = get_db(read_only=True)
    rows = con.execute(
        "SELECT id, name, match_config, action, deliver_to, hook_command, cooldown_minutes, enabled FROM alerts ORDER BY name"
    ).fetchall()
    con.close()

    alerts = []
    for r in rows:
        match_config = {}
        if r[2]:
            try:
                match_config = json.loads(r[2]) if isinstance(r[2], str) else r[2]
            except (json.JSONDecodeError, TypeError):
                match_config = {}

        alerts.append({
            "id": r[0],
            "name": r[1],
            "match_config": match_config,
            "action": r[3] or "deliver",
            "deliver_to": r[4],
            "hook_command": r[5],
            "cooldown_minutes": r[6] or 30,
            "enabled": bool(r[7]),
        })

    return {"alerts": alerts, "total": len(alerts)}


def main():
    init_feed_system()

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            request = json.loads(line)
        except json.JSONDecodeError as e:
            response = {
                "jsonrpc": "2.0",
                "id": None,
                "error": {"code": -32700, "message": f"Parse error: {e}"}
            }
            print(json.dumps(response), flush=True)
            continue

        response = handle_request(request)
        if response is not None:
            print(json.dumps(response), flush=True)


if __name__ == "__main__":
    main()
