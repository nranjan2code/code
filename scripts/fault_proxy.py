#!/usr/bin/env python3
"""Fault-injecting reverse proxy for OpenCode Zen.

Forwards POST /chat/completions to https://opencode.ai/zen/v1, streaming SSE
back. Fault mode is read from FAULT_MODE_FILE before each request:

  pass          forward normally
  429           respond 429 retry-after:1 without touching upstream
  503           respond 503 overloaded without touching upstream
  truncate:N    stream upstream but cut the body after N bytes (no [DONE])
  garbage       emit malformed SSE frames (bad json, random binary)
  slow          forward but sleep 3s before each chunk (starves watchdogs)
  hang          accept, then stall the response indefinitely

GET /hits -> JSON counters. GET /mode -> current mode.
POST /mode <text> -> set mode (also writable via the file directly).
"""
import json
import sys
import threading
import time
from http.client import HTTPSConnection
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MODE_FILE = "/tmp/fault_mode"
UPSTREAM_HOST = "opencode.ai"
LOCK = threading.Lock()
STATS = {"total": 0, "injected": {}, "completed": 0, "upstream_errors": 0}


def mode():
    try:
        with open(MODE_FILE) as f:
            return f.read().strip()
    except OSError:
        return "pass"


def note(kind):
    with LOCK:
        STATS["total"] += 1
        if kind != "pass":
            STATS["injected"][kind] = STATS["injected"].get(kind, 0) + 1


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        if self.path == "/hits":
            body = json.dumps(STATS).encode()
        elif self.path == "/mode":
            body = mode().encode()
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        if self.path == "/mode":
            length = int(self.headers.get("content-length", 0))
            new = self.rfile.read(length).decode().strip()
            with open(MODE_FILE, "w") as f:
                f.write(new)
            self.send_response(200)
            self.send_header("content-length", "2")
            self.end_headers()
            self.wfile.write(b"ok")
            return

        m = mode()
        note(m)
        length = int(self.headers.get("content-length", 0))
        body = self.rfile.read(length)

        if m == "429":
            self._fixed(429, b'{"error":{"message":"injected rate limit","type":"rate_limit_error"}}', {"retry-after": "1"})
            return
        if m == "503":
            self._fixed(503, b'{"error":{"message":"injected overload"}}')
            return
        if m == "hang":
            time.sleep(300)
            return

        try:
            conn = HTTPSConnection(UPSTREAM_HOST, timeout=60)
            conn.request("POST", "/zen/v1" + self.path, body=body, headers={
                "authorization": self.headers.get("authorization", ""),
                "content-type": "application/json",
                "accept": self.headers.get("accept", "text/event-stream"),
            })
            resp = conn.getresponse()
        except Exception as e:
            with LOCK:
                STATS["upstream_errors"] += 1
            self._fixed(502, json.dumps({"error": {"message": f"proxy upstream: {e}"}}).encode())
            return

        if resp.status != 200:
            payload = resp.read()
            self.send_response(resp.status)
            self.send_header("content-length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            return

        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("connection", "close")
        self.end_headers()

        try:
            if m == "garbage":
                self.wfile.write(b"data: {not json at all\n\n")
                self.wfile.write(b"data: [CORRUPTED\xff\xfe\n\n")
                self.wfile.flush()
                time.sleep(2)
                self.close_connection = True
                return
            sent = 0
            limit = None
            if m.startswith("truncate:"):
                limit = int(m.split(":")[1])
            while True:
                chunk = resp.read(512)
                if not chunk:
                    break
                if limit is not None and sent + len(chunk) > limit:
                    chunk = chunk[: limit - sent]
                    self.wfile.write(chunk)
                    self.wfile.flush()
                    self.close_connection = True  # hard cut, no [DONE]
                    return
                self.wfile.write(chunk)
                self.wfile.flush()
                sent += len(chunk)
                if m == "slow":
                    time.sleep(3)
            with LOCK:
                STATS["completed"] += 1
            self.close_connection = True
        except (BrokenPipeError, ConnectionResetError):
            self.close_connection = True

    def _fixed(self, status, payload, extra=None):
        self.send_response(status)
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *a):
        pass


if __name__ == "__main__":
    port = int(sys.argv[1])
    ThreadingHTTPServer(("127.0.0.1", port), H).serve_forever()
