#!/usr/bin/env python3
"""Minimal MCP stdio server for offline tests: one echo tool + one failing tool."""
import json
import sys

TOOLS = [
    {
        "name": "echo",
        "description": "Echo back the input text",
        "inputSchema": {
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
        },
    },
    {
        "name": "boom",
        "description": "Always fails",
        "inputSchema": {"type": "object"},
    },
]


def send(msg):
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        send({
            "jsonrpc": "2.0",
            "id": rid,
            "result": {
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "fake-mcp", "version": "0.1.0"},
            },
        })
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": rid, "result": {"tools": TOOLS}})
    elif method == "tools/call":
        params = req.get("params", {})
        name = params.get("name")
        if name == "echo":
            text = params.get("arguments", {}).get("text", "")
            send({
                "jsonrpc": "2.0",
                "id": rid,
                "result": {"content": [{"type": "text", "text": f"echo: {text}"}]},
            })
        else:
            send({
                "jsonrpc": "2.0",
                "id": rid,
                "result": {
                    "isError": True,
                    "content": [{"type": "text", "text": "boom failed on purpose"}],
                },
            })
    elif rid is not None:
        send({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": f"unknown method {method}"}})
