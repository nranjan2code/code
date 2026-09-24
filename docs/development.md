# Developing and testing Vak live

How to build, run and check Vak during development, including a live run
against a real model in the browser. Everything here was learned the hard
way in a session; follow it instead of rediscovering it. The contract
rules it serves are in `AGENTS.md` ("Verification before every commit",
"Acceptance-workspace contract", "Live development runs").

## 1. Build

```bash
cargo build -p vak -p vak-server --bins
```

This builds `target/debug/vak` and `target/debug/vak-tool-worker`. The
worker binary is not optional: every model tool, every target verifier and
every Office review runs in it (invariant 14), and server tests that
freeze, verify or promote a candidate fail without it.

## 2. Frontends and the embedded bundle

`vak-server` embeds `crates/vak-client-ui/dist-web` at compile time, and
its build script refuses a bundle whose `.src-manifest` does not match
`src/`. After any client change:

```bash
npm --prefix crates/vak-client-ui run build
```

then rebuild the server binary (step 1) and restart it. A running server
keeps serving the bundle it was compiled with.

When two branches both rebuilt `dist-web`, a merge conflicts on hashed
asset names. Never hand-merge them: resolve the source files, delete
`dist-web`, run the build, and stage the result.

## 3. Checks beyond `cargo test`

- **Node tests** for pure client logic: `node crates/vak-client-ui/tests/<name>.mjs`.
  Node strips TypeScript itself, so they import `../src/*.ts` directly.
  Run them all with a loop over `tests/*.mjs`.
- **Component harnesses** render one component against fixed data, with
  no server: for example `crates/vak-client-ui/tests/office-review.html`.
  Start Vite for the client (`VAK_HOST=web npx vite --port <port>` in
  `crates/vak-client-ui`), open `/app/tests/<name>.html` (the dev server
  serves under `/app/`, not `/`), and call `window.runChecks()`, which
  throws on the first failed check and returns `"<name>: ok"`. Check phone
  width too (375 px) and that `document.documentElement.scrollWidth` does
  not exceed the viewport.
- **A known flaky test**: `vak-flow`'s
  `chain_runs_with_substitution_and_merge` has failed once under full
  workspace load and passed in isolation. Rerun a failure in isolation
  before blaming your change, and say which you did.

## 4. A live run against a real model

Use this when a change has to be seen working end to end: a tool the
model calls, Review, acceptance, undo.

### Where and how it runs

- **Workspace:** a disposable directory under `/tmp` (the
  acceptance-workspace contract), never the source checkout.
- **Port:** anything but `8901`, which the installed service normally
  holds (`lsof -nP -iTCP:8901 -sTCP:LISTEN`). The examples use `8931`.
- **Data home and provider credentials:** run against the default data
  home. A copy made with `VAK_HOME=<copy>` does not bring the provider
  connection with it (seen 2026-09-24: "anthropic is not connected"), so
  the model cannot be reached. The dev build against the real data home
  only adds: a new session history keyed by the disposable workspace, its
  sandbox candidates and records, and cost-log entries for the model calls.
  Do not pass `--gateway`, so no channel traffic is involved.
- **Never open credential files** (`credentials.enc`, `.credential_key`,
  the credential index) or read ledgers in the real data home. Nothing in a
  live run needs them, and Claude Code's auto mode refuses such reads.
  Inspect state through the app and its HTTP API instead (below).

### Signing in

A server generates its sign-in token per process and does not print it
when there is no terminal. Pin a throwaway token for the test server
instead, kept outside git with owner-only permissions:

```bash
umask 077 && python3 -c "import secrets;print(secrets.token_urlsafe(32))" > /tmp/vak-live/test-gateway-token
```

Start the server with it (from the checkout or worktree you are testing):

```bash
VAK_GATEWAY_TOKEN=$(cat /tmp/vak-live/test-gateway-token) target/debug/vak serve -C /tmp/vak-live/workspace --port 8931 --trust
```

and print a signed-in link to the web client:

```bash
VAK_GATEWAY_TOKEN=$(cat /tmp/vak-live/test-gateway-token) target/debug/vak open app --print --port 8931 -C /tmp/vak-live/workspace
```

The link carries `?token=`, which works only on loopback (invariant 34).
Opening `/app` without it shows the client but every call answers 401.

In Claude Code, run the server through the preview tools rather than a
shell: add an entry to `.claude/launch.json` in the main checkout, for
example

```json
{
  "name": "vak-live",
  "runtimeExecutable": "sh",
  "runtimeArgs": ["-c", "VAK_GATEWAY_TOKEN=$(cat /tmp/vak-live/test-gateway-token) exec <checkout>/target/debug/vak serve -C /tmp/vak-live/workspace --port 8931 --trust"],
  "port": 8931
}
```

Wrapping the server in `script` to get a terminal does not work there (no
TTY on stdin).

### In the browser

- **Keep one tab per server origin.** Each client tab holds several
  long-lived event streams, and browsers allow six HTTP/1.1 connections per
  host across all tabs, so with a second or third tab open, ordinary
  requests hang with no error. If a button does nothing, close the other
  tabs on that origin before debugging the button.
- The server answers the API directly with the token as a bearer header,
  which is the way to inspect state:

  ```bash
  curl -s -H "Authorization: Bearer $(cat /tmp/vak-live/test-gateway-token)" http://127.0.0.1:8931/sessions/<session-id>/sandbox/records
  ```

- A draft a tool wrote lands in `.vak/scratch/<agent>/<call-id>/` inside
  the workspace. Review opens from the result card's **Review draft**, or
  from Workbench → **Build activity** → **Review candidate**. Accepting
  writes the workspace; **Undo acceptance** is offered only in the page
  that accepted it (a reload loses the offer until that is fixed), and
  the server's `POST /sessions/<id>/sandbox/promotions/<candidate-id>/undo`
  undoes it regardless.

### Test files

For Office work, `crates/vak-ooxml/src/fixtures.rs` (feature `fixtures`)
builds minimal Word, Excel, PowerPoint and Visio packages for tests. For a
live run, a realistic file is better; `openpyxl` and `python-docx` are not
installed here, so write the parts with Python's `zipfile` (shared
strings, a styles part, and a formula with a cached value is enough to be
realistic for a workbook). A file saved by Office itself is better still
when one is available.

### Cleaning up

Stop the server. Frozen candidates are write-protected on purpose, so a
test directory that holds any needs `chmod -R u+w` before `rm -rf`. Delete
any copy of a data home you made: it contains a copy of the encrypted
credentials.

## 5. Working in a git worktree

- A worktree has its own `target/`, so its first build is a full one.
- `node_modules` can be a symlink to the main checkout's when the
  lockfiles match; exclude it locally (`.git/info/exclude` of the main
  repository) rather than in `.gitignore`.
- `.claude/launch.json` is read from the main checkout; entries for a
  worktree point at the worktree's paths.
- Merge the base branch into the feature branch and verify there before
  fast-forwarding the base. When the base checkout has uncommitted work
  that touches the same files, ask the owner whether to commit, stash or
  wait; never stash or commit someone's work on a guess.
