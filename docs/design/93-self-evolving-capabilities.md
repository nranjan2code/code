Self-Evolving Capabilities
==========================

Status: in progress: E1 built (E1a 2026-10-14: a skill's program runs as
a tool under its own sandbox; E1b 2026-10-15: declared hosts through the
egress proxy, pinned Python requirements installed by their own run); E2
built 2026-10-15 (a program's own tests run by the runtime in a run of
their own, kept per digest); E3 to E9 not built. Agreed with the
maintainer on 2026-10-14. Paths below describe targets and may not exist yet. Replaces
[43-governed-self-evolution](43-governed-self-evolution.md).

## 1. What this is

When a person asks for something Vakyartha cannot yet do well, the Agent
looks at what it has already built, reuses it, extends it, or builds it,
proves it works by running it, and keeps it so the next request finds it.
Over time the install gains abilities from use rather than from new
releases. The Rust runtime, the permission engine, the broker and the
sandbox stay as they are: evolution adds abilities and never widens
authority.

The pieces are already in the tree; what is missing is the one kind of
thing that joins them:

| Piece | Where it is today |
|---|---|
| Durable knowledge | `remember`, memory, `session_search` (doc 26) |
| Written procedures | `propose_skill` → review → `SKILL.md` in the Shared layer (doc 26, `vak-core/src/learning.rs`) |
| Finding what exists | capability registry and epochs (doc 41), `find_tools`, the data catalog (doc 73 §9) |
| Running new code safely | the broker worker and its sandboxes (invariant 14, doc 25) |
| Proof from running, not words | invariant 33 (`Observed` over `Asserted`) |
| Versions, parents, Review, undo | the Library (doc 82), file state (doc 90), candidates (doc 54) |
| No restart for a new ability | level-triggered registry (invariant 31) |
| What works, measured | `vak_judge::measure`, `Promote` (doc 88 §7, C7) |
| Repeated work | triggers (doc 73 §7, `vak_core::triggers`) |
| **Runnable abilities the Agent writes** | **missing** |

Today a skill is prose: next time the model reads the instructions and
writes the code again. This doc makes a skill able to carry a program
the runtime runs as a tool.

### Non-goals

- Changing the Rust runtime, its crates or its invariants. Evolution
  stays above the runtime (D2). Runtime changes are made by people (and
  coding sessions they drive), through the gate.
- Working around a provider's refusals. "Never gives up" is a design
  property: when a tool is missing, build one; when an approach fails,
  try another. Refusals belong to the model provider and are not routed
  around.
- A second plugin system. A capability is a skill; a plugin may ship one
  (doc 39) the same way it ships skills today.

## 2. Decisions

Made by the maintainer on 2026-10-14 (`docs/decisions.md`):

- **D1 Tiered admission.** An Agent-made capability that runs sandboxed,
  reaches no network and only reads the workspace (and writes only its
  own drafts) goes live without asking. Anything that needs the network,
  a secret, an outside effect or a write outside drafts waits in Review.
- **D2 Above the runtime.** Agents evolve skills, capabilities, flows,
  automations and presentation packs. Never the Rust core.
- **D3 Extend, then tell.** On a near match the Agent makes a new version
  whose parent is the old one, on its own, and the answer says so
  ("Used *Split a bill*", "Improved *Split a bill* to version 3"). The old
  version stays; undo is one step.
- **D4 Agent first, then Shared.** A capability lives in the Agent that
  built it until its evidence earns promotion to the Shared layer, where
  every Agent and workspace finds it.
- **D5 Scripts in the sandbox.** A capability's program is a Python,
  shell or Node script with a declared argument schema, run by the broker
  worker under the session's sandbox.
- **D6 Tests and real use.** Promotion to Shared needs the capability's
  own tests passing as the runtime ran them, and real use that succeeded,
  both read from receipts.
- **D7 On demand plus a review of failures.** Agents build and improve
  while answering requests. A scheduled automation also reads failure
  receipts and proposes fixes as reviewable candidates.
- **D8 A compact index plus search.** The prompt carries one line per
  capability the Agent can use; full schemas come through `find_tools`.

Decided on the maintainer's delegation (2026-10-14, "make the best
decisions"); the maintainer may reopen any of them:

- **R1 One kind.** A capability is a skill with an optional executable
  part, not a second kind beside skills (invariant 30). `propose_skill`
  remains the one write path; a skill with no program behaves exactly as
  today.
- **R2 Promotion by observed states, not counts.** "N successful turns"
  would be a tuned number ("No word lists, no tuned limits"). Promotion
  is offered when the current version (a) passed its tests as the runtime
  ran them, (b) succeeded in at least one turn other than the turn that
  built it, and (c) has no failed call since it was made. Retirement is
  the mirror: a version whose calls fail is shown its failures, a fix is
  proposed, and a version that fails again after the fix is withdrawn
  for that Agent. No count anywhere; the ladder is `vak_control::Ladder`.
- **R3 Dependencies install once, as their own run.** A capability names
  its packages with exact versions and hashes (`requirements.txt` with
  `--require-hashes`; Node lockfiles are open, see the tracker). Installing is a separate
  install run in the broker worker whose one reachable host is the
  package index, through the egress proxy; it fills the space's execution
  cache. A capability that needs an install is Review the first time (it
  reaches the network); every later call runs offline from the cache,
  with the network denied. A changed lock is a new version and a new
  install.
- **R4 A capability shows through the existing surfaces; it draws
  nothing itself** (§3.1). Its result is data that the model shows with
  the card forms; files it writes to drafts are previewed like any draft;
  a skill may carry declarative presentation packs.
- **R5 Card data may come from the call.** A big table or chart that the
  model has to copy from a result into a card argument is where calls
  break (doc 89). A card may name the capability call whose result holds
  its data, and the loop fills the card from that call, the way
  `Tool::output_of_command` fills a terminal card. The model still
  decides whether and where a card is shown (stage E9).
- **R6 Office files and PDFs only through `office_apply`.** A capability
  never delivers an Office file or a PDF (detected by content, invariant
  39): such a file in its drafts is refused with "return the data and
  write the document with office_apply". It may read them through
  `doc_read`'s projection passed as an argument, never parse them itself.
- **R7 Unattended work is always Review.** A capability may propose an
  automation (a `Trigger`) that calls it; the trigger is Review whatever
  the capability's tier, because nobody watches it run.

## 3. A capability

A capability is a skill directory, versioned in the Library:

```
<name>/
  SKILL.md          frontmatter + instructions (as today)
  capability.json   the program's declaration (optional)
  run.py | run.sh | run.mjs     the program
  tests/            cases the runtime runs: input JSON → expected checks
  requirements.txt | package.json   pinned dependencies (optional)
```

`capability.json` (`vak_tools::capability::Declaration`), JSON because the
arguments are a JSON Schema; `SKILL.md`'s frontmatter is unchanged and
names the tool:

```json
{
  "entry": "run.py",
  "arguments": {"type": "object", "properties": {
    "amount": {"type": "number", "description": "The bill before tip"}
  }},
  "hosts": [],
  "writes": "none",
  "requirements": null
}
```

- `entry`: the script inside the skill directory; `.py` runs with
  `python3`, `.sh` with `sh`, `.mjs`/`.js` with `node`, found on the PATH
  a scrubbed command gets.
- `hosts`: outbound hosts, each an exact name (HTTPS and HTTP) or
  `name:port`, no wildcard, scheme or path; empty is no network at all.
  A call with hosts starts an egress proxy of its own
  (`vak_tools::egress`, the one MCP servers use) and the program's sandbox
  may connect to that proxy's loopback port alone
  (`Sandbox::proxied_variant`); the proxy settings reach it as
  `HTTPS_PROXY` and its kin.
- `writes`: `none` or `drafts` (its execution's drafts, named
  `.vak/scratch/<agent>/<execution>/…`, given to it as `VAK_DRAFTS`).
- `requirements`: `{"file": "requirements.txt", "hosts": [...]}`, a pip
  requirements file every package of which is pinned with its hash, and
  the package index hosts it is fetched from (named by the declaration,
  never by Vak). Python programs only (a shell or Node program declaring
  them is refused). The environment is content-addressed, under
  `cache_home()/capability-envs/<digest of the file and the interpreter>`,
  so one install serves every program pinning the same packages. When it
  is missing, the call first runs `python3 -m venv` and `pip install
  --require-hashes` in a worker whose sandbox writes only a staging folder
  and reaches only those hosts through their own proxy; the host moves the
  folder into place (a call that installed it first wins). Until it is
  installed the tool declares `Reach::Network` and `leaves_machine`, so
  the permission engine asks on the first call in a restricted mode;
  afterwards the program runs offline from the environment.
- `timeout_ms` is the runtime's: added to the advertised schema, filled
  by the loop (`Tool::time_limit_argument`), never passed to the
  program; a declaration that names it is refused.

What the program reaches is derived, never declared twice: `writes:
none` is `Reach::ReadPath`, `writes: drafts` is `Reach::WritePath`, and a
host, or dependencies not installed yet, is `Reach::Network`. Its results
are private (it reads the workspace), and private and untrusted when it
declares hosts. The tool is named as the skill and
is offered only when no tool of the turn has that name. Its digest is
over every file in the skill directory: a call to a program changed
since the turn froze it is refused, and its new version is offered from
the next turn.

Rules:

- **The declaration is the permission.** `hosts` and `writes` become
  the tool's `Tool::permission` (and, with hosts, `leaves_machine`); the engine reads them like any other tool's (AGENTS.md
  "A tool's permission is what it declares"). Nothing is decided by the
  capability's name.
- **One schema, flat and portable.** The arguments obey
  `every_card_schema_is_flat_and_portable`'s rules and go strict where
  the wire enforces them (doc 89). A schema that fails those checks is
  refused when the capability is proposed, with what to fix.
- **The program speaks JSON.** Arguments on stdin, one JSON value on
  stdout (with `writes: drafts`, the result is `{"result": …, "drafts":
  [names]}`), a non-zero exit as a failure; stderr is kept as the call's
  diagnostic and stdout and stderr stream to the Workbench.
- **Its own sandbox.** The broker runs it as `WorkerTask::Capability` in a
  worker wrapped by a sandbox built from the declaration, never the
  session's: reads the system, the workspace, the skill directory and the
  interpreter's installation; writes only its temporary directory, its
  cache and (when declared) its drafts; no network. A system with no OS
  backend refuses to run it. The result is a normal `ToolOutput`; the program cannot
  claim a card, an artifact or provenance beyond what its declaration
  allows.
- **Provenance.** Its results are labelled from its declaration
  (`Tool::provenance`): a capability with `hosts` returns untrusted
  content, and one that reads the workspace returns private content.
  Invariant 43 applies unchanged.

### 3.1 What a capability can show

Everything already on screen stays the one way to show it (invariant 30).
A capability adds no renderer and mounts no HTML of its own.

| To show | How | Exists today |
|---|---|---|
| Figures, tables, charts, steps, comparisons, sources… | its JSON result; the model shows it with the card forms (`vak_delivery::FORMS`, doc 92), filling a card's data from the call when it names one (R5) | the card tools, yes; filling from a call, E9 |
| A page, a dashboard, an interactive UI | an HTML file (with its assets) written to drafts; shown by a `file_preview` card on the preview origin (invariant 35, doc 66), opened in the Canvas | yes |
| An image, a chart image, audio, video | a file in drafts; a `media` card names it | yes |
| A document, a sheet, slides, a PDF | the data in its result; the model writes the file with `office_apply`, which goes through Review (R6, invariant 39) | yes |
| A new look for a card kind | a presentation pack in the skill's `presentation/` directory: declarative, compiled into the closed primitives, runs no code (doc 91) | packs, yes; packs in an Agent-made skill, E3 |
| A scheduled result | a proposed `Trigger` that calls the capability, always Review (R7) | triggers, yes; proposing one, E8 |

A skill directory may therefore carry, beside its program:

```
  presentation/<pack>.json   declarative packs (doc 91), versioned with the skill
```

A pack in an Agent-made skill follows doc 91's rules (scope, channel
allow and deny, inventory) and becomes active with the version that
carries it; disabling or retiring the version removes its kinds at the
next turn, and cards already drawn fall back to their block's renderer.

## 4. The loop

```
request
  │  the prompt's capability index (D8) + find_tools / catalog search
  ▼
reuse ── fits as is ─────────────────────────────▶ call it
  │ close
  ▼
extend: propose_skill with `parent` = that version  ┐
  │ nothing                                         ├─▶ verify ─▶ admit ─▶ call it
  ▼                                                 │
build: propose_skill with no parent                 ┘
                                                     ▼
                         receipts → measure → promote / fix / retire
```

1. **Find.** The Agent reads the index; `find_tools` ranks capabilities
   with the rest of the deferred set. The catalog finds a capability by
   what it does and where it came from (a new catalog source, §6).
2. **Decide.** Reuse, extend or build is the model's choice, and it is
   recorded: the plan's deliverable names it, and the `propose_skill`
   call names the parent when it extends. Nothing reads the request's
   words to decide it.
3. **Verify.** The runtime runs `tests/` (`Core::verify_skill_program`,
   `vak skills verify <name>`) in a run of its own, each case through
   the very path a call takes (`CapabilityTool`, its declared sandbox and
   proxies, any first install inside that run), within the operator's
   command limit (`[control.limits]`). A case is `tests/<name>.json`:
   the `arguments` a call sends and either the result it must `expect`
   (every key of an object, every item of an array in order, equal
   scalars) or `"fails": true`. Each case's verdict, and where its
   answer differed, is what the runtime observed; the report is kept as
   a Document per program digest (`AgentScope::capability_checks`,
   `Core::skill_program_verification`) and the run settles completed or
   failed as its cases did. A program with no tests, or a case with
   neither `expect` nor `fails`, is refused when the program loads, with
   what to add. E3 attaches the report to the Library version.
4. **Admit.** The tier (§5) decides whether the version goes live at
   once or waits in Review. Going live publishes a registry epoch; the
   next turn boundary of every live session of that Agent sees it
   (invariant 31).
5. **Use.** A call is an ordinary tool call through `authorize`, the
   permission engine and the broker (invariant 16). The worker runs the
   program as a `WorkerTask::Capability { skill, version, args }` with the
   execution environment of invariant 35 and the capability's pinned
   dependencies in the space's execution cache.
6. **Measure.** Each call's verdict and fault are already decisions and
   receipts; `vak_judge::measure` grows a per-capability-version measure
   from them.
7. **Promote, fix or retire** by R2.

The answer says what happened in plain words (§9.1). D3 means extending
never waits for the person; the version's parent and the Library's undo
make it reversible.

## 5. Tiers

The tier is computed from the declaration and the provenance of the turn
that proposed it, never from words:

| Tier | When | What happens |
|---|---|---|
| **Live** | `hosts` empty, `writes` drafts or none, `reach` within read and command, tests passed, and the proposing turn held no untrusted content | live for the Agent at the next turn boundary |
| **Review** | anything else: a host, a secret, an outside effect, a write outside drafts, a failed or missing test, or a turn that held untrusted content | a candidate in Review; the inbox says so |
| **Live (packs)** | a presentation pack, which runs no code and reaches nothing | live with its version, whatever the program's tier |
| **Refused** | a declaration the session itself could not use (wider than its mode), a schema that fails the portability checks, a program with no tests, an Office file or PDF in its drafts (R6) | refused with what to fix |

- **Untrusted content pushes to Review.** A turn that read a web page, a
  mail or another person's words can be steered into writing a
  capability. Whatever it proposes waits for the owner, however narrow
  (invariant 43's join, applied to proposals).
- **No widening.** A capability can never declare more than the session
  that proposed it could do. The engine checks the declaration against
  the session's mode when it is proposed and again at every call.
- **The tier is the owner's setting.** Settings → Capabilities offers
  "Let Agents switch on new abilities that only work with your files"
  (on by default, per D1) and nothing wider. Turning it off sends every
  proposal to Review.
- **FullAccess** does not change the tier rules: a capability is code
  that will run again later, unattended, not one command the owner saw.

AGENTS.md's outcome-runtime paragraph ("hooks/tools … require human
review") changes, in the stage that builds the gate (E3), to say that an
Agent-made capability in the Live tier is admitted inside its envelope.

## 6. Storage and the data rules

| What | Where | Kind |
|---|---|---|
| A proposal (draft version) | the Agent's skill-proposal Documents (`AgentScope::skill_proposals`), as today | Document |
| A version's files | a Library artifact per capability; each version names its parent (`vak_core::artifacts`, `Artifacts::transaction`) | artifact versions |
| Test results | rows on the version's thread (`library::note_review`) with the worker task's run id | Library rows |
| Live for an Agent | a Document in the Agent home: capability → admitted version | Document |
| Live for everyone (Shared) | the Shared layer's skills directory (`Core::shared_skills_dir`), installed from the version by promotion | Shared layer |
| Measurements | derived from decisions and receipts, never stored separately (doc 88 §7) | query |

Every rule in AGENTS.md "The data rules" holds; the ones that bite here:

- Every new durable file is declared in `vak_core::state::REGISTRY`
  (`agents/{agent}/capabilities/…`).
- The capability is a **catalog source** in the same change: a node per
  capability and version, edges to the turn and run that proposed it and
  to the parent version, so search and lineage find it (invariant 37's
  audience applies).
- Verification and the idle review are **runs** (`Core::runs().open`);
  test results and proposals are keyed by run id; their spans carry the
  trace key (invariant 40).
- A capability built from a conversation names it in `derived_from`.
  Erasing that conversation erases the proposal drafts derived from it.
  A promoted capability is the owner's own artifact and stays; the
  receipt says so (invariant 42).
- Promotion writes the Shared skill through the one install path
  (`learning::promote` grows the program and tests); nothing copies
  files of its own.

## 7. Discovery and the prompt

- The index is one line per capability the Agent can use (name and
  description), built from the registry's epoch. It sits with the tool
  index in the byte-stable prefix and changes only when an epoch
  publishes, i.e. at a turn boundary (invariant 36).
- Full schemas load through `find_tools`, as deferred tools do today
  (`ToolSurface`). As built in E1a, a program's tool declares no `serves`
  domains, so `build_tool_surface` loads it on every turn (undeclared
  fails open); E4 carries the skill's `serves:` onto its tool so it
  defers. A capability declares `serves` so the surface can
  rank it; undeclared fails open (doc 41, `capability/domain.rs`).
- The prompt seed gains no instruction naming any capability. What the
  model needs to know about building is the `propose_skill` tool's own
  description and schema (prompt-seed budget, doc 07).

## 8. Idle review

A built-in automation per Agent (a `Trigger`, claimed through
`triggers::claim_due`, owned by the Agent, D7):

1. Reads the decisions and receipts since its last run (a cursor,
   `vak_session::cursors`) for calls to the Agent's capabilities that
   failed.
2. For each failing version, proposes a fix as a new version whose
   parent is the failing one, in a copy environment (invariant 38's
   scheduled-work rule), and verifies it.
3. Lands every proposal in **Review**, never Live: nobody watched it
   being written.
4. Says what it did in the inbox; when it cannot run, says why
   (`RoutineFailed`).

Its spend is under the FinOps budget like any run. It is off for a new
Agent until the owner turns it on in the Agent's settings (the first
one is offered once, in the inbox, after a capability first fails).

## 9. Experience

The product has no users yet, so screens that get in the way are
rewritten, not patched (maintainer, 2026-10-14). All
strings follow doc 75 §7: people see "abilities" and "skills", never
"capability", "epoch", "schema", "tier" or hashes, unless Show technical
details is on. Each screen is done when it holds in the running app at
1440 × 900 and 390 × 844, light and dark (AGENTS.md "Visual refresh").

### 9.1 In the conversation (the projection)

- **A typed record, not narration.** Using, improving, building and
  retiring are recorded as an audit entry (`CapabilityActivity`) beside
  the call, and `vak-server`'s projection draws it as one quiet line
  under the answer: "Used **Split a bill**", "Improved **Split a bill**
  (version 3)", "Learned **Split a bill**", "**Split a bill** is waiting
  for your review". The model's prose is not required to say it, and
  no surface reads the prose to find it.
- The line opens a **capability sheet**: what it does, versions with
  the change from the last one, tests and their results, recent uses,
  and actions (Use the earlier version, Review, Turn off). With technical
  details on, the sheet shows the program, the declaration and run ids.
- A call to a capability shows in the Workbench like any execution
  (invariant 35): live output, status, files it wrote.
- A channel, webhook or inbox summary carries the same line in its text
  form (`projection::text_with_run_cards`'s neighbour for activities).

### 9.2 Review

Capability candidates join Review beside file candidates (doc 54,
`/library/{id}/versions/{version}`):

- What it will be allowed to do, in plain words from the declaration
  ("Reads files in this project", "Connects to api.example.com"),
  first and largest.
- Why it is here instead of live ("It connects to the internet",
  "It was written after reading a web page").
- The tests as the runtime ran them, and the diff from its parent.
- Approve (goes live for that Agent), Approve for all Agents (promotes
  to Shared, offered only when R2 holds), Decline (the proposal is kept
  as declined history), Ask for changes (a message to the Agent).

### 9.3 Library

The Library page (`LibraryPage.tsx`) gains an **Abilities** view, the
canonical place a person sees what their Agents have learned:

- One row per capability: name, what it does, who learned it, where it
  is (this Agent, all Agents, waiting), last used, and its state in
  plain words (Working, Failing, Turned off, Retired).
- Filter by Agent and by state; search through the catalog.
- The row opens the capability sheet (§9.1). Version history is the
  Library's own; restoring an earlier version makes a new version.

If the current Library page cannot carry a second kind of item cleanly,
its layout is redone for both, not given a separate page.

### 9.4 Settings

Settings → Capabilities (`Settings.tsx`, today a list of skills, plugins
and connections) is rewritten around three sections:

1. **Learned by your Agents**: the Abilities summary with a link to the
   Library view, and the two switches: "Let Agents switch on new
   abilities that only work with your files" (D1) and, per Agent, "Look
   at failures and suggest fixes" (§8).
2. **Installed**: plugins and their skills, as today.
3. **Connections**: MCP servers and channels, as today.

An Agent's own settings show its abilities and its review switch, so
"what did this Agent learn" is answered where the Agent is.

### 9.5 Inbox

New entry kinds, each opening its sheet or Review: an ability waiting
for review, an ability improved by the review automation, an ability
withdrawn after failing, and an offer to promote one to all Agents.
`NeedsYou` counts the review entries.

### 9.6 Admin console

- **#/capabilities** (new): the full inventory across Agents and the
  Shared layer, origin (built-in, plugin, Agent-made), admitted version,
  tier, declaration, lineage (the Lineage view, from the catalog), and
  revoke, which takes effect at once (invariant 31).
- **#/measurements**: a per-capability-version table beside the per-model
  one: calls, failures by fault, corrections, and which versions R2 would
  promote or retire.
- **#/inbox** shows the new entry kinds; **#/runs** shows verification and
  review runs like any run.

### 9.7 CLI

`vak skills` grows the same verbs (list with state, show, versions,
review, approve, decline, promote, turn off, retire), over the same
routes; there is no second implementation.

## 10. Security

Threats and what holds them (doc 24's order):

- **A steered proposal.** Injected text asks the Agent to write a
  capability that leaks data. Holds: the proposing turn's untrusted
  label sends it to Review (§5); a capability with `hosts` is Review; at
  run time, invariant 43 makes any outbound call in a turn holding
  private and untrusted content ask.
- **A capability lying about itself.** The declaration says no network;
  the program opens a socket. Holds: the sandbox enforces the
  declaration, not the program (network denied unless `hosts`, then only
  through the egress proxy); a backend that cannot confine it refuses
  to run it (invariant 13).
- **Escalation through evolution.** A version declares more than its
  parent. Holds: the tier is computed for every version; a wider
  declaration is Review, a declaration wider than the session is
  Refused.
- **Evolving the gate.** Nothing in this loop can edit the tier rules,
  the permission engine, the broker, the sandbox or this design; they
  are Rust and change only through the gate (D2).
- **Persistence after revoke.** Revoking a capability, an Agent or the
  Shared layer's copy takes effect immediately and fails closed
  (invariants 11 and 31).
- **Secrets.** A capability never reads a secret from its environment
  (invariant 12). A capability that needs a credential declares it and is
  Review; injection is recipient-scoped like an MCP server's.

## 11. Stages

Each stage is done only when its "done when" holds, with the full gate
green, checked live on the dev route (and, for screens, in the browser at
both sizes and themes). The tracker is `docs/plans/open-items.md`
"Self-evolving capabilities".

| Stage | What | Done when |
|---|---|---|
| **E0** | This design agreed; doc 43 superseded | decisions logged; tracker has E1–E8 |
| **E1** | Executable skill: `run` frontmatter, the declaration → `Tool` declarations, `WorkerTask::Capability`, JSON in/out, pinned dependencies in the execution cache | a hand-written capability in the Shared layer is found by `find_tools`, called through the broker, refused the network it did not declare (tested on Seatbelt) |
| **E2** | Verification: `tests/` run through the call path in a run of its own, results kept per program digest (Library rows in E3) | a failing test case is reported per case with the run id; a capability with a program and no tests is refused with what to add |
| **E3** | Proposal and tiers: `propose_skill` takes a program, tests and `parent`; versions as Library artifacts; the tier gate; the Agent-live Document; registry epoch; AGENTS.md paragraph updated | a live turn builds a capability, it goes live and the next turn calls it; a turn that fetched a page first lands its proposal in Review |
| **E4** | Catalog source and the prompt index; `find_tools` ranking | a later conversation finds and reuses a capability built in an earlier one, and extends it with `parent` set |
| **E5** | Measurement and R2: per-version measure, promotion offer, fix-then-withdraw ladder | a capability that fails is withdrawn for the Agent after its proposed fix fails again; one that meets R2 is offered for all Agents |
| **E6** | Experience: §9.1–§9.5 in the client (projection line, sheet, Review, Library Abilities, Settings, inbox) | each screen's acceptance at 1440 × 900 and 390 × 844, light and dark, in the running app |
| **E7** | Admin and CLI: §9.6, §9.7 | revoke from #/capabilities stops the next call; CLI verbs over the same routes |
| **E8** | Idle review automation (§8) | a failing capability gets a fix proposed in Review overnight, with a run record and an inbox entry; a review that cannot run says why |

| **E9** | Card data from a call (R5): a card names a capability call; the loop fills its rows, points or figures from that call's result, generalising `Tool::output_of_command` | a table card of a few hundred rows from a capability call is shown with every value equal to the result, and a card naming a call that did not run is refused |

E1 to E3 are the core; E6 can start after E3 against real records.

## 12. Open questions

- **Which interpreters.** Python, shell and Node are what the
  development machines have; a box without Node refuses `run.mjs`
  capabilities with a clear reason (never a silent skip).
- **Sharing a capability between owners.** Out of scope for a single-
  owner install (`docs/decisions.md`, single-owner app); a capability
  leaves only as a plugin package (doc 39).
- **Flows.** A static flow (`vak-flow`) is the remaining kind D2 names
  without a proposal path here; decided after E5, from whether real
  capabilities need composing beyond one turn's calls.
