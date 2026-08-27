# 05 — Configuration and secrets

`vak-config` is the only configuration authority. `vak-runtime` is the only
caller allowed to apply configuration to a run or mutate it on disk.

## Layers

```text
<data_home>/config.toml
        < <project>/.vakcoder/project.toml
        < process environment
        < explicit CLI/client command
```

Project configuration is accepted only after the project root is registered.
Unknown keys are reported as warnings and ignored. Writes use a revision and
atomic replacement, so concurrent clients receive a conflict instead of
silently overwriting one another.

## Stored values

Runtime configuration covers provider endpoint/name, discovered model choice,
permission mode, sandbox, limits, and connection profiles. Provider credentials
live in `<data_home>/.env` (0600) or the process environment. Secrets
are never stored in TOML, passed as ambient worker environment, logged, or
written to session entries.

## Admission

When a session or run is created, Runtime resolves effective configuration and
records an immutable `SessionContract`. Changing configuration affects new
admissions only. A permission or sandbox change also advances the capability
epoch, cancels old-epoch work, and rejects old approvals before reopening
admission.

## Inspection

`vakcoder config dump` and the admin/TUI/desktop settings views call the Runtime
query endpoint. They show effective non-secret values and redact credentials.
