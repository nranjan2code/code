"""The speed simulator (Simulator tab): what a turn costs, stage by stage, as
the model's output speed moves from today's tens of tokens a second to tens of
thousands.

Two kinds of input, kept apart on the page:
- MEASURED: how real turns spent their time, from the content-free telemetry
  of one install (span durations only). It is a dated observation, not read
  on every build, because the blueprint measures the tree and its history and
  a clean checkout has no telemetry.
- CODE: the constants and per-token paths that decide what happens at high
  speed, read from the tree on every build, so the page follows the code and
  a moved or removed one lands in "Needs review".

Everything else the page computes (the screen's cost per token, reading
speed) is labelled an estimate where it is shown.
"""

import json
import os
import re

import measure as ms

# 48 turns, 375 steps, 153 tool calls on one Mac install, 2026-10-15 to
# 2026-10-18, routes openai-responses and ollama: span.close durations from
# <logs>/vak-server.jsonl (turn, step, dispatch, tool_call). No content.
MEASURED = {
    "as_of": "2026-10-18",
    "turns": 48, "steps": 375, "tool_calls": 153,
    "share_model": 0.81, "share_tools": 0.12, "share_runtime": 0.02,
    "turn_p50_ms": 22387, "turn_p90_ms": 102991,
    "step_p50_ms": 3291, "dispatch_p50_ms": 3192, "dispatch_hosted_p10_ms": 2163,
    "runtime_per_step_ms": 40, "runtime_per_turn_ms": 250,
    "tool_p50_ms": 199,
    "tools": {"bash": 60, "webfetch": 455, "glob": 59, "doc_read": 38, "write": 168},
    "steps_per_turn": {"p50": 4, "p90": 17, "max": 51},
    "tools_per_turn": {"p50": 2, "p90": 13},
    "log_bytes_per_step": 1842, "log_lines_per_step": 4.1,
}

# What the page reads from the code: (key, file, pattern, what it decides).
# A pattern's first group, when it has one, is the value.
CODE = [
    ("ring", "crates/vak-server/src/events.rs", r"const CAPACITY: usize = (\d+);",
     "Events a reconnecting client can catch up on (EventBus ring)"),
    ("wait_tick", "crates/vak-agent/src/workers_tool.rs", r"const WAIT_TICK: Duration = Duration::from_millis\((\d+)\);",
     "How often a parent looks at a worker it waits on"),
    ("delta", "crates/vak-server/src/client_events.rs", r"Some\(ClientEvent::TextDelta \{ delta \}\)",
     "One wire frame per text delta"),
    ("append", "crates/vak-client-ui/src/store.ts", r"function appendToLast\(",
     "The screen copies the item list and the whole text on every delta"),
    ("snapshot", "crates/vak-llm/src/stream.rs", r"TextDelta \{\s*delta: String,\s*partial: AssistantMessage",
     "Every in-process delta carries the whole message so far (invariant 4)"),
    ("breaker", "crates/vak-agent/src/circuit.rs", r"cooldown: Duration::from_secs\((\d+)\)",
     "Circuit breaker cooldown after blind failures"),
    ("day_cap", "crates/vak-config/src/lib.rs", r"pub max_day_usd: Option<f64>",
     "The operator's day budget; admission stops when it is spent"),
    ("rate_gate", "crates/vak-llm/src/rate_limit.rs", r"pub struct RateLimitGate",
     "Requests wait on the provider's allowance and honour Retry-After (invariant 7)"),
    ("fsync", "crates/vak-storage/src/records.rs", r"self\.file\.sync_data\(\)",
     "Each durable append ends in a sync to disk"),
]


def line_of(text, pos):
    return text.count("\n", 0, pos) + 1


def code_facts():
    facts, review = {}, []
    for key, path, pattern, what in CODE:
        full = os.path.join(ms.ROOT, path)
        text = open(full, encoding="utf-8").read() if os.path.isfile(full) else ""
        m = re.search(pattern, text)
        if not m:
            review.append(f"simulator: {key} not found in {path} (harness/simulator.py CODE)")
            continue
        facts[key] = {"file": path, "line": line_of(text, m.start()), "what": what,
                      "value": int(m.group(1)) if m.groups() else None}
    # how many adapter sites clone the whole message into each delta
    src = os.path.join(ms.ROOT, "crates/vak-llm/src")
    clones = sum(open(os.path.join(src, f), encoding="utf-8").read().count("partial: self.message.clone()")
                 for f in sorted(os.listdir(src)) if f.endswith(".rs"))
    if "snapshot" in facts:
        facts["snapshot"]["value"] = clones
    return facts, review


# What drives each subsystem's work as the model gets faster, by hand, each
# with its reason and the checks (CODE keys or the page's infrastructure
# checks) that say when it bends or breaks. A subsystem not listed does work
# per action, not per token, step or turn, and speed alone does not load it.
#   token: work per streamed token      step: work per model step or call
#   turn: work per turn                 spend: tokens bought
#   timer: a fixed wait in code         person: waits on a person
SPEED = {
    "llm": ("token", "Parses every streamed token and copies the whole message into each delta; asks the provider's allowance", ["snapshot", "allow"]),
    "server": ("token", "Numbers every delta into the event ring and sends one frame per delta", ["ring", "frames"]),
    "client": ("token", "Applies every delta to its store and draws the text again", ["append"]),
    "desktop": ("token", "Hosts the workspace client, so it draws every delta too", ["append"]),
    "terminal": ("token", "Draws the same stream as a client of the server", ["frames"]),
    "cli": ("token", "Prints the stream as it arrives", []),
    "admin": ("step", "Its event hub carries every session's events", ["frames"]),
    "loop": ("step", "Runs every step: dispatch, repair, cards, stop gate; a blind failure opens the breaker", ["breaker"]),
    "context": ("step", "Plans and assembles every request", []),
    "control": ("step", "Writes a decision at every decision point", []),
    "permission": ("step", "Decides every call; a provenance ask waits on a person", ["person"]),
    "sandbox": ("step", "Starts a sandboxed worker for every tool call", []),
    "tools": ("step", "Runs every built-in tool call in the worker", []),
    "cards": ("step", "Checks and records every card call", []),
    "hooks": ("step", "Runs around every tool call", []),
    "mcp": ("step", "Brokers every integration call", []),
    "recall": ("step", "Answers recall calls from the catalog", []),
    "flows": ("step", "Walks a flow's nodes as steps", []),
    "ledger": ("step", "Seals and syncs a frame for every entry", ["fsync", "disk"]),
    "storage": ("step", "Stores every object and moves refs under the writer epoch", ["fsync"]),
    "catalog": ("step", "Tails every ledger to index turns, calls and runs", ["disk"]),
    "telemetry": ("step", "Writes about four content-free lines a step", ["logs"]),
    "turn": ("turn", "Admits every turn: agent, trace key, route ladder", []),
    "runs": ("turn", "Opens and settles a run for every turn and records every effect", []),
    "intent": ("turn", "Reads every request into strands", []),
    "prompt": ("turn", "Resolves the prompt layers every turn", []),
    "capabilities": ("turn", "Binds a capability epoch every turn", []),
    "commitments": ("turn", "Brackets an episode per strand", []),
    "goal": ("turn", "Keeps goal state across turns", []),
    "memory": ("turn", "Recalls and writes notes as turns go", []),
    "checkpoints": ("turn", "Checkpoints the workspace around changes", ["disk"]),
    "security": ("turn", "Records every denial and escalation", []),
    "operations": ("turn", "Projects receipts and incidents from the record", []),
    "bus": ("turn", "Carries events between agents", []),
    "delivery": ("turn", "Renders each answer for its surface", []),
    "gateway": ("turn", "Routes each channel message to a turn", ["allow"]),
    "workers": ("timer", "A parent looks at a worker it waits on by a fixed tick", ["wait_tick"]),
    "finops": ("spend", "Prices every receipt and stops admission when the day budget is spent", ["budget"]),
    "inbox": ("person", "Holds what waits for the owner", ["person"]),
    "library": ("person", "Review and promotion wait for the owner", ["person"]),
    "coworking": ("person", "Shared conversations move at the people's pace", ["person"]),
    "voice": ("person", "Speech plays at speaking speed, whatever the model's", ["person"]),
}


def speed_map(model):
    known = {x["id"] for x in model["subsystems"]}
    review = [f"simulator: SPEED names {sid}, which is not a subsystem (harness/simulator.py)" for sid in SPEED if sid not in known]
    out = {sid: {"driver": d, "why": why, "checks": checks} for sid, (d, why, checks) in SPEED.items() if sid in known}
    return out, review


def data(model):
    facts, review = code_facts()
    speed, speed_review = speed_map(model)
    model["review"].extend(review + speed_review)
    names = {x["id"]: x["name"] for x in model["subsystems"]}
    model["simulator"] = {"measured": MEASURED, "code": facts, "subsystems": speed, "names": names}
    return json.dumps(model["simulator"], ensure_ascii=False).replace("</", "<\\/")
