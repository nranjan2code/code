#!/usr/bin/env python3
"""Minimal MCP stdio server for offline tests: one echo tool + one failing tool."""
import json
import os
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
    {
        "name": "secret_result",
        "description": "Returns a diagnostic result",
        "inputSchema": {"type": "object"},
    },
    {
        "name": "net_probe",
        "description": "Tries host:port directly, then through HTTPS_PROXY",
        "inputSchema": {
            "type": "object",
            "properties": {"target": {"type": "string"}},
            "required": ["target"],
        },
    },
]


def net_probe(target):
    import socket

    host, port = target.rsplit(":", 1)
    said = []
    try:
        socket.create_connection((host, int(port)), timeout=2).close()
        said.append("direct:open")
    except OSError:
        said.append("direct:denied")
    proxy = os.getenv("HTTPS_PROXY", "")
    if not proxy:
        said.append("proxy:none")
        return " ".join(said)
    proxy_host, proxy_port = proxy.split("//")[-1].rsplit(":", 1)
    try:
        conn = socket.create_connection((proxy_host, int(proxy_port)), timeout=2)
        conn.sendall(f"CONNECT {target} HTTP/1.1\r\n\r\n".encode())
        said.append("proxy:" + conn.recv(64).decode().split("\r\n")[0])
        conn.close()
    except OSError:
        said.append("proxy:failed")
    return " ".join(said)


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
        elif name == "net_probe":
            target = params.get("arguments", {}).get("target", "")
            send({
                "jsonrpc": "2.0",
                "id": rid,
                "result": {"content": [{"type": "text", "text": net_probe(target)}]},
            })
        elif name == "secret_result":
            send({
                "jsonrpc": "2.0",
                "id": rid,
                "result": {"content": [{"type": "text", "text": f"diagnostic: {os.getenv('MCP_TEST_SECRET', 'none')}"}]},
            })
        else:
            send({
                "jsonrpc": "2.0",
                "id": rid,
                "result": {
                    "isError": True,
                    "content": [{"type": "text", "text": f"boom failed on purpose: {os.getenv('MCP_TEST_SECRET', 'none')}"}],
                },
            })
    elif rid is not None:
        send({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": f"unknown method {method}"}})
