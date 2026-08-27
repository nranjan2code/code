# 12 — Evaluation

`vak-runtime` exposes a deterministic evaluation boundary. The request is a
JSON array of case objects; Runtime counts valid object cases, records an
`eval.completed` audit event, and returns passed/failed totals.

The CLI adapter is:

```text
vakcoder eval
vakcoder eval --live --provider <provider> --model <discovered-model>
```

Provider credentials remain in the Runtime secret service. Evaluation output
is typed JSON and never changes session history or permission state.
