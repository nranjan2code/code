#!/usr/bin/env python3
"""Drive the vakcoder TUI through a real PTY and assert on rendered output."""
import os
import pty
import select
import subprocess
import sys
import time

BIN = sys.argv[1]
ENV = {
    **os.environ,
    "ANTHROPIC_API_KEY": "test",
    "VAKCODER_ANTHROPIC_BASE_URL": sys.argv[2],
    "VAKCODER_HOME": "/tmp/vak-smoke/home",
    "TERM": "xterm-256color",
}

master, slave = pty.openpty()
proc = subprocess.Popen(
    [BIN],
    stdin=slave,
    stdout=slave,
    stderr=slave,
    env=ENV,
    close_fds=True,
)
os.close(slave)

out = b""


def pump(seconds):
    global out
    deadline = time.time() + seconds
    while time.time() < deadline:
        r, _, _ = select.select([master], [], [], 0.1)
        if master in r:
            try:
                chunk = os.read(master, 65536)
            except OSError:
                return False
            if not chunk:
                return False
            out += chunk
    return True


def send(text):
    os.write(master, text.encode())


try:
    pump(1.0)
    send("/help\r")
    pump(0.8)
    send("/cost\r")
    pump(0.8)
    send("hello world\r")
    pump(1.5)
    send("/exit\r")
    deadline = time.time() + 10
    while time.time() < deadline:
        r, _, _ = select.select([master], [], [], 0.2)
        if master in r:
            try:
                chunk = os.read(master, 65536)
            except OSError:
                break
            if not chunk:
                break
            out += chunk
        elif proc.poll() is not None:
            break
    code = proc.wait(timeout=5)
except Exception as e:
    print(f"driver error: {e}")
    proc.kill()
    code = proc.returncode

import re

clean = re.sub(rb"\x1b\[[0-9;]*[a-zA-Z]", b"", out)
clean = clean.replace(b"\r\n", b"\n").replace(b"\r", b"\n")

text = clean.decode("utf-8", "replace")
checks = {
    "banner version": "0.1.0" in text,
    "help lists /model": "/model <name>" in text,
    "cost line": "tokens in 0 / out 0" in text,
    "prompt echo": "hello world" in text,
    "tool started": "▸ bash" in text,
    "tool succeeded": "✓ bash" in text,
    "final answer streamed": "smoke-ok. Task complete." in text,
    "bye": "bye" in text,
}
failed = [k for k, ok in checks.items() if not ok]
for k, ok in checks.items():
    print(f"{'PASS' if ok else 'FAIL'}: {k}")
print(f"exit code: {code}")
sys.exit(0 if not failed and code == 0 else 1)
