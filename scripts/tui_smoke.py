#!/usr/bin/env python3
"""Drive the vakcoder TUI through a real PTY and assert on rendered output."""
import os
import pty
import select
import subprocess
import sys
import time
import fcntl
import struct
import termios

BIN = sys.argv[1]
ENV = {
    **os.environ,
    "ANTHROPIC_API_KEY": "test",
    "VAKCODER_PROVIDER": "anthropic",
    "VAKCODER_MODEL": "claude-sonnet-4-5",
    "VAKCODER_ANTHROPIC_BASE_URL": sys.argv[2],
    "VAKCODER_HOME": "/tmp/vak-smoke/home",
    "TERM": "xterm-256color",
}

master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 55, 110, 0, 0))
proc = subprocess.Popen(
    [BIN, "tui", "--trust"],
    stdin=slave,
    stdout=slave,
    stderr=slave,
    env=ENV,
    cwd="/tmp/vak-smoke/project",
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
    send("\x1b")
    pump(0.4)
    send("/config\r")
    pump(0.8)
    send("\x1b[6~")
    pump(0.5)
    send("t")
    pump(0.7)
    send("\x1b")
    pump(0.4)
    send("/config\r")
    pump(0.7)
    send("f")
    pump(0.7)
    send("\x1b")
    pump(0.4)
    send("/provider\r")
    pump(0.8)
    send("\x1b")
    pump(0.4)
    send("/theme\r")
    pump(0.8)
    send("teenage")
    pump(0.6)
    send("\x13")
    pump(0.8)
    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 42, 140, 0, 0))
    pump(0.5)
    send("/cost\r")
    pump(0.8)
    send("hello world\r")
    pump(2.0)
    send("\x1ba")
    pump(0.5)
    send("y")
    pump(2.0)
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
    "banner version": "0.3.0" in text,
    "help lists configuration": "/model" in text and "/provider" in text and "/config" in text,
    "help modal": "help · commands" in text and "PROMPT INPUT" in text,
    "settings modal": "settings" in text and "RELIABILITY" in text and "PATHS" in text,
    "feature explorer": "feature explorer" in text and "AGENT RUNTIME" in text,
    "provider picker": "choose provider" in text and "ANTHROPIC_API_KEY" in text,
    "theme picker": "choose theme" in text and "Teenage Engineering-inspired" in text,
    "theme saved": "saved theme teenage" in text,
    "composer restored": "Ln 1, Col 1" in text,
    "alternate screen": "?1049h" in text and "?1049l" in text,
    "responsive resize": "\u2500" * 120 in text,
    "cost line": "tokens in 0 / out 0" in text,
    "prompt echo": "hello world" in text,
    "approval card": "approval · bash" in text and "echo smoke-ok" in text,
    "approval answered": "✓ allowed bash" in text,
    "tool started": "tool bash" in text,
    "tool succeeded": "✓ bash" in text,
    "final answer streamed": "smoke-ok. Task complete." in text,
    "bye": "bye" in text,
}
failed = [k for k, ok in checks.items() if not ok]
for k, ok in checks.items():
    print(f"{'PASS' if ok else 'FAIL'}: {k}")
print(f"exit code: {code}")
if failed:
    print("--- sanitized PTY capture ---")
    print(text)
sys.exit(0 if not failed and code == 0 else 1)
