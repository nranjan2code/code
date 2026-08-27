# 09 — Extensibility boundary

Extensions are data and adapters around the Runtime; they do not create a
second authority.

## Tools

`vak-tools` exposes the brokered filesystem, Bash, glob/grep, web fetch, and
browser tools. Runtime supplies the registered project root, cancellation
token, permission mode, sandbox, and bounded operational environment. A tool
result is typed data with an `is_error` flag and is recorded in the run path.

## Flows

Project flow definitions live under `.vakcoder/flows`. Runtime discovers and
validates them, then admits each run with the same session contract and
authorizes every effectful node through the broker.

## Skills and templates

Runtime stores skill records and their review state. Delivery templates are
bounded data files loaded by `vak-delivery`; they can format approved slots but
cannot execute code or access credentials.

## Rule

An extension may add a typed adapter or declarative data, but it must submit
mutations through Runtime and preserve the append-only session and audit
contracts.
