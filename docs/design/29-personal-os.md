# 29 — Runtime personal services

Personal data is handled by Runtime services so every surface sees the same
records and authorization decisions.

## Services

- project-scoped and profile memory with append, amend, and forget operations;
- task records with enable/disable, update, and deletion transitions;
- approval records and addressed resolution;
- inbox records with explicit acknowledgement;
- skill proposal listing, promotion, and rejection;
- checkpoint capture/list/restore and backup export/import;
- flow discovery/check/run and deterministic eval accounting;
- provider key lifecycle and live model discovery;
- durable delivery-job state and append-only operational audit.

## Ownership

`vak-runtime` composes `vak-services`, `vak-storage`, `vak-session`,
`vak-config`, and `vak-agent`. The HTTP server, typed client, TUI, desktop,
admin UI, and CLI call Runtime methods; none open the data home directly.

## Safety

Memory, task text, inbox payloads, skills, and delivery payloads are data. They
cannot grant permissions, select a sandbox, inject credentials, or bypass an
approval. Runtime performs capability-epoch and broker checks immediately
before every effect and writes an audit record for control-plane mutations.
