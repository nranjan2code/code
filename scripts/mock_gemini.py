#!/usr/bin/env python3
"""Mock Google Gemini streamGenerateContent SSE server for offline smoke tests."""
import http.server
import json
import sys

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8790


def chunk(parts, finish=None, usage=None):
    out = {"candidates": [{"content": {"parts": parts, "role": "model"}, "finishReason": finish}]}
    if usage:
        out["usageMetadata"] = usage
    return f"data: {json.dumps(out)}\n\n".encode()


class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("content-length", 0))
        body = json.loads(self.rfile.read(length) or b"{}")
        has_fn_resp = any(
            "functionResponse" in p
            for c in body.get("contents", [])
            for p in c.get("parts", [])
        )
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("connection", "close")
        self.end_headers()
        if not has_fn_resp:
            self.wfile.write(chunk([{"text": "Gemini checking. "}]))
            self.wfile.write(chunk(
                [{"functionCall": {"name": "bash", "args": {"command": "echo gemini-ok"}}}],
                finish="STOP",
                usage={"promptTokenCount": 44, "candidatesTokenCount": 11},
            ))
        else:
            self.wfile.write(chunk([{"text": "Saw the response. Done."}], finish="STOP"))
            self.wfile.write(chunk([], usage={"promptTokenCount": 70, "candidatesTokenCount": 8}))
        self.wfile.flush()

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    http.server.HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
