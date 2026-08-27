# 10 — Runtime flows

Flows are project-local TOML definitions discovered by Runtime. The current
contract supports listing and validating definitions, then starting a normal
Runtime run after validation. The definition is metadata for admission; its
nodes are not a second execution engine.

## Format

```toml
name = "verify"

[[nodes]]
id = "tests"
kind = "bash"
command = "cargo test"
```

Runtime reports the flow name, source path, validity, and node count. A missing
or malformed TOML file is returned as invalid data and cannot be run.

## Interfaces

```text
vakcoder flow list
vakcoder flow check <name>
vakcoder flow run <name>
```

All three commands use `vak-client` and the authenticated server. `flow run`
validates the named definition and submits the ordinary Runtime prompt
`run flow <name>`, reusing normal session/run admission, cancellation, event
streaming, and audit. It cannot create a private state owner or bypass the
broker.
