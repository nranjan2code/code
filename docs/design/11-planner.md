# 11 — Runtime flow admission

The greenfield CLI exposes only flows registered with Runtime. A surface never
constructs or executes a private plan graph.

```text
vakcoder flow list
        │
        ├── flow check <name>  ──► Runtime validation
        └── flow run <name>    ──► Runtime validation → normal run admission
```

Runtime validates the flow definition, then submits a normal prompt run. The
run freezes the session contract, authorizes effects through the broker, and
streams the same delta plus snapshot events used by normal prompts. There is no
parallel planner or node executor. Resume requests identify the existing
session and are checked against the stored project and capability epoch before
dispatch.

Flow definitions are read from registered project configuration. Runtime is the
only read/write path, so CLI, TUI, desktop, admin, and external adapters observe
identical flow-definition state and audit records.
