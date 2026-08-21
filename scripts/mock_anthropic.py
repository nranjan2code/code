#!/usr/bin/env python3
"""Mock Anthropic SSE server for offline smoke tests."""
import http.server
import json
import os
import sys

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8787
TOOL_NAME = os.environ.get("MOCK_TOOL", "bash")
TOOL_INPUT = (
    {"action": "list"} if TOOL_NAME == "mcp" else {"command": "echo smoke-ok"}
)

TOOL_TURN = {
    "type": "message_start",
    "message": {"model": "claude-sonnet-4-5", "usage": {"input_tokens": 100, "output_tokens": 10}},
}
TOOL_BLOCKS = [
    {"type": "content_block_start", "index": 0, "content_block": {"type": "text"}},
    {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Let me check. "}},
    {"type": "content_block_stop", "index": 0},
    {"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": "toolu_smoke1", "name": TOOL_NAME}},
    {"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": json.dumps(TOOL_INPUT)}},
    {"type": "content_block_stop", "index": 1},
    {"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 40}},
]
FINAL_TEXT = [
    {"type": "content_block_start", "index": 0, "content_block": {"type": "text"}},
    {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "The command printed: smoke-ok. Task complete."}},
    {"type": "content_block_stop", "index": 0},
    {"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 60}},
]


def sse(events, final_type="message_stop"):
    out = []
    for e in events:
        name = e["type"]
        out.append(f"event: {name}\ndata: {json.dumps(e)}\n\n")
    out.append(f"event: message_stop\ndata: {{\"type\": \"{final_type}\"}}\n\n")
    return "".join(out).encode()


class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("content-length", 0))
        body = json.loads(self.rfile.read(length) or b"{}")
        n_user_msgs = sum(1 for m in body.get("messages", []) if m["role"] == "user")
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("connection", "close")
        self.end_headers()
        if n_user_msgs <= 1:
            self.wfile.write(sse([TOOL_TURN] + TOOL_BLOCKS))
        else:
            self.wfile.write(sse(FINAL_TEXT))
        self.wfile.flush()

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    http.server.HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
