#!/usr/bin/env python3
"""Mock OpenAI Responses API SSE server for offline smoke tests."""
import http.server
import json
import sys

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8789


def ev(etype, extra=None):
    out = {"type": etype}
    if extra:
        out.update(extra)
    return f"event: {etype}\ndata: {json.dumps(out)}\n\n".encode()


class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("content-length", 0))
        body = json.loads(self.rfile.read(length) or b"{}")
        types = [i.get("type") for i in body.get("input", [])]
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("connection", "close")
        self.end_headers()
        if "function_call_output" not in types:
            # first call: text + function_call item
            self.wfile.write(ev("response.output_text.delta", {"delta": "Responses API checking. "}))
            self.wfile.write(ev("response.output_item.added", {
                "item": {"type": "function_call", "id": "fc_1", "call_id": "call_smoke",
                         "name": "bash", "arguments": ""},
            }))
            self.wfile.write(ev("response.function_call_arguments.delta", {
                "item_id": "fc_1", "delta": "{\"command\": \"echo resp-ok\"}",
            }))
            self.wfile.write(ev("response.completed", {
                "response": {"usage": {"input_tokens": 55, "output_tokens": 12}},
            }))
        else:
            self.wfile.write(ev("response.output_text.delta", {"delta": "Saw the output. Done."}))
            self.wfile.write(ev("response.completed", {
                "response": {"usage": {"input_tokens": 90, "output_tokens": 18}},
            }))
        self.wfile.flush()

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    http.server.HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
