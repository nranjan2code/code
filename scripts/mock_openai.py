#!/usr/bin/env python3
"""Mock OpenAI chat-completions SSE server for offline smoke tests."""
import http.server
import json
import sys

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8788


def chunk(delta, finish=None, usage=None):
    out = {"id": "chatcmpl-1", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}
    if usage:
        out["usage"] = usage
    return f"data: {json.dumps(out)}\n\n".encode()


class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("content-length", 0))
        body = json.loads(self.rfile.read(length) or b"{}")
        roles = [m.get("role") for m in body.get("messages", [])]
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("connection", "close")
        self.end_headers()
        if "tool" not in roles:
            # first call: text + tool_call
            self.wfile.write(chunk({"role": "assistant", "content": "Checking via OpenAI. "}))
            self.wfile.write(chunk({"tool_calls": [{"index": 0, "id": "call_smoke", "type": "function", "function": {"name": "bash", "arguments": ""}}]}))
            self.wfile.write(chunk({"tool_calls": [{"index": 0, "function": {"arguments": "{\"command\": \"echo openai-smoke\"}"}}]}))
            self.wfile.write(chunk({}, finish="tool_calls"))
            self.wfile.write(chunk({}, usage={"prompt_tokens": 21, "completion_tokens": 13}))
        else:
            self.wfile.write(chunk({"role": "assistant", "content": "Saw the tool result. Done."}))
            self.wfile.write(chunk({}, finish="stop"))
            self.wfile.write(chunk({}, usage={"prompt_tokens": 40, "completion_tokens": 9}))
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    http.server.HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
