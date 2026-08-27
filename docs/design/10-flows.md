# 10 — Runtime flows

Flows are project-local TOML definitions discovered by Runtime. The current
contract supports listing and validating definitions, then starting a normal
Runtime run associated with the selected flow.

## Format

```toml
name = "verify"

[[nodes]]
id = "tests"
kind = "bash"
command = "cargo test"
```

Runtime reports the flow name, source path, validity, and node count. A missing
or malformed TOML file is returned as invalid data; it is not silently
executed.

## Interfaces

```text
vakcoder flow list
vakcoder flow check <name>
vakcoder flow run <name>
```

All three commands use `vak-client` and the authenticated server. `flow run`
reuses normal session/run admission, cancellation, event streaming, and audit;
it cannot create a private state owner or bypass the broker.
