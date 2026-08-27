# 11 — Runtime flow execution

The greenfield CLI exposes only flows registered with Runtime. A surface never
constructs or executes a private plan graph.

```text
vakcoder flow list
        │
        ├── flow check <name>  ──► Runtime validation
        └── flow run <name>    ──► Runtime admission → brokered execution
```

Runtime validates the flow, freezes the session contract, authorizes every
effectful node, persists node and run outcomes, and streams the same delta plus
snapshot events used by normal prompts. A failed node is a typed terminal
outcome; no alternate plan is silently substituted. Resume requests identify
the existing session and are checked against the stored project and capability
epoch before dispatch.

Flow definitions are read from registered project configuration. Runtime is the
only read/write path, so CLI, TUI, desktop, admin, and channel surfaces observe
identical flow state and audit records.
