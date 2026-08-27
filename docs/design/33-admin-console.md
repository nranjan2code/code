# 33 — Admin console

The admin console is the browser presentation of the same authenticated
Runtime API used by CLI, TUI, and desktop. External delivery adapters consume
Runtime packets separately. The console is embedded in
`vak-server`; it does not add a data store or privileged execution path.

## Architecture

```text
vak-admin-ui (SolidJS/Vite, committed dist)
              │ /auth/login then secured Runtime routes
              ▼
        vak-server auth + SSE
              │
              ▼
         vak-runtime
```

The SPA has no credentials other than its HttpOnly session cookie and never
reads files. Mutations are limited to the Runtime routes that are exposed by
the server; the Runtime validates and audits each one before dispatch.

## Authentication

Browser login posts the one-time gateway token to `/auth/login`; the server
sets an HttpOnly, SameSite=Strict `vakcoder_session` cookie. CLI and native
clients use the Runtime token as a bearer credential through `vak-client`.
Only Admin static assets and `/auth/login` are public: `/health`, `/version`,
diagnostics, and every Runtime resource require authentication. A browser that
has no valid cookie shows an explicit recovery state directing the user to open
a fresh Admin Console; it never presents authentication failure as an
indefinite connection spinner.

## Views and commands

The console exposes live projects, sessions, runs, tasks, workspace memory,
inbox records, configuration diagnostics, and run-event streams. It can cancel
runs and inspect the same Runtime-owned records as the other adapters. Session
creation, prompts, provider discovery, file access, checkpoints, flows, evals,
and backup operations remain on their dedicated typed clients and CLI surface.

## Events

`vak-server` emits ordered run events and global operational events through
SSE. Every stream event includes its delta and snapshot where applicable. A
client that falls behind reconnects and reloads the authoritative snapshot;
the browser never reconstructs state by guessing from presentation events.

## Verification

Admin smoke tests use a disposable data home and one Runtime: login, load the
project/session/run/task/memory/inbox views, stream a run, cancel it, and
confirm that every displayed record comes from the Runtime. `npm run build`
produces the committed embedded assets.
