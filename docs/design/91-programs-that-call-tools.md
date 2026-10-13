# 91 — Programs that call tools

Status: proposal, 2026-10-13. Nothing here is built. Written at the
maintainer's request after a design comparison with Nous Research's Hermes
Agent (read from its source at `6e7b83a2`) found one capability Vak has no
answer to: the model writing one program that makes many tool calls and
returns only the answer. Builds on doc 88 (the control plane), doc 89
(constrained tool arguments), doc 85 (the turn graph) and doc 68 (the
context engine). No step starts until the maintainer takes the decisions in
§4 and says which stage to begin.

## 1. The gap

A fan-out job (find 40 files, read each, pull one field, compare them)
costs Vak one model step per call today, and every result passes through
the context on its way to the next step. The model reads 40 results it
needs one field of, and the turn's input grows with each step.

Hermes answers this with `execute_code`: the model writes a Python script,
the script calls tools over RPC, and only what it prints returns to the
model. Hermes' own SECURITY.md (§2.2) says that script runs as a host
subprocess, outside its terminal backend's isolation, alongside MCP,
plugins and hooks. Vak takes the saving and not that boundary.

## 2. The shape

- One loop-owned tool, `program`, with one argument, `code`. The loop
  defines it, as it defines `plan`, `recall` and `task`, so it is
  recognised by name (AGENTS.md, "No word lists, no tuned limits").
- The code runs in the broker worker as a new `WorkerTask::ProgramRun`,
  which moves the worker protocol (`PROTOCOL_VERSION` in
  `crates/vak-tools/src/broker.rs`) from 5 to 6. It runs read-only,
  network denied, rooted at the workspace, whatever the session's mode, as
  `VerifyTargets` and `OfficeReview` do (invariant 14).
- The script has no file, network, process or clock API. Its only effect
  is `await tools.<name>(args)`, a request on the worker's channel back to
  the broker.
- The broker handles each inner call as a call the model made: the one
  argument reader (`vak_tools::read_arguments`), the tool's
  `fill_defaults`, the schema check, `authorize` through `PermissionEngine`
  and the approver, execution in resource-claim waves, a child trace key
  (`TraceKey::child`), and a `Decision` entry for its verdict. A program
  therefore never reaches more than the model could by calling the same
  tools one by one.
- An inner call's error comes back to the script as a value carrying its
  `vak_control::Fault`, so the script can handle it. A program that fails
  is `ToolOutput::error(Fault::…)`, naming where the fault lies.
- Only what the script returns is model-visible. Every inner call is in
  the ledger and reachable by `recall`.

## 3. Invariants it keeps

| Invariant | How |
|---|---|
| 1, model-visible means logged | The `program` call (its code untruncated in the arguments) and its result are ordinary entries. Inner calls are a new audit-only entry, `EntryPayload::ProgramCall`, naming the outer `tool_use_id` and the turn (`Entry::at_turn`); `derive_messages()` never reads it. |
| 2, append-only | Inner calls are appended as they finish; nothing is rewritten when the program ends. |
| 5, abort keeps partial output | Cancellation threads into the worker and stops the engine; inner calls already made stay recorded and recallable. |
| 14, broker boundary | The worker gets the code and a call channel, never the policy engine, approvals, credentials or session store. |
| 15, unattended surfaces fail closed | An Ask inside a program on a surface nobody answers is a denial, returned to the script as a value. |
| 16, authorize before dispatch | Inner calls go through `authorize`; an Ask blocks the script until the approver answers. |
| 36, nothing cut blind | The returned value is handled as any long tool result is; no character truncation. Inner results stay out of the working set and are reached by `recall`. |
| 40, trace key | Each inner call carries a child of the `program` call's key; the catalog traces it to the run. |
| Declared reach | A tool joins programs only by a new declaration, `Tool::programmable` (default false), forwarded by every wrapper; never a list of names. Card tools, `plan`, `task`, `work` and anything that acts outside Vak (an effect) never declare it. |
| No tuned limits | Wall time is an operator limit in `[control.limits]` (`TimeLimits::program_secs`, shown in Settings beside its default), and `program` names it through `Tool::time_limit_argument`. There is no cap on inner calls; the operator's turn limit still bounds the run. |

## 4. Decisions to take first

1. **Script language.** Recommended: JavaScript in an engine embedded in
   the worker, pinned exactly in the workspace manifest, with no host
   bindings. It needs no installed interpreter, starts in milliseconds and
   has `Promise.all` for parallel calls, which the broker maps onto
   resource-claim waves. Rejected: the machine's Python (its version
   differs per machine, its standard library reaches files and network,
   and a sandbox would be all that stood between them), and a language of
   Vak's own (models write an unfamiliar language badly, which is a
   contract fault this design should not introduce).
2. **First reach.** Recommended: only tools whose declared reach reads
   (`read`, `grep`, `glob`, `doc_read`, `webfetch`, `session_search`, and
   MCP tools that declare it). Writes follow in P6, each a draft, so
   Review still decides what reaches the workspace.
3. **How the model learns the bindings.** Recommended: a typed signature
   per programmable tool derived from its `call_schema`, served through
   `find_tools`, not added to the byte-stable prefix, so the prompt seed's
   budget and cache stay as they are.
4. **Long code as one argument.** Doc 89 measured that long string
   arguments are the ones that break. Recommended: `program` is a strict
   tool where the wire enforces a schema, the regeneration path covers the
   rest, and the form is measured with `scripts/measure_tool_form.py` on
   Vak's captured requests before P4 ships.

## 5. Stages

Each stage ships whole with its tests and docs, and replaces nothing it
leaves behind (invariant 30).

| Stage | Work | Done when |
|---|---|---|
| P0 · Baseline | Capture real fan-out turns in a `/tmp` workspace on at least two routes; record steps, input tokens and wall time per task in `docs/audits/`. Take the decisions in §4. | The baseline is committed and this document's status line records the decisions. |
| P1 · Declarations and ledger | `Tool::programmable`; `EntryPayload::ProgramCall` (traced, `at_turn`, parent call id, a large result as a tenant object); a catalog source for it; `docs/reference/records.md` regenerated. | `derive_messages()` is byte-identical with and without inner entries; `every_ledger_row_type_is_traced` passes. |
| P2 · Worker runtime | The engine behind `WorkerTask::ProgramRun`: no host APIs, the call channel, cancellation, the operator's deadline; protocol 6. | An escape corpus (file, network, process, prototype and global tricks, endless loops, huge returns) fails closed on macOS and Linux, and a fuzz target runs in CI. |
| P3 · Broker dispatch | Inner calls through reader, `fill_defaults`, `authorize`, execution in waves, approvals, child trace keys, `Decision` entries, typed faults returned to the script. | Tests in every permission mode, an unattended channel included, show a program cannot call what its session could not. |
| P4 · The loop tool | Register `program`; its result enters context, inner results stay out and are reachable by `recall`; the plan check (`vak_judge::plan`) reads inner receipts; workers spawned by `task` get it too. | The P0 tasks run live on two routes with fewer steps and fewer input tokens than the baseline, and no result lost. |
| P5 · Surfaces | One call in the transcript that opens to its inner calls; Workbench shows them live; the admin Run detail shows them in the span tree. Both client bundles rebuilt. | Checked in a browser at 1440 × 900 and 390 × 844, light and dark. |
| P6 · Later | Write reach (each write a draft for Review), MCP tools by their own declaration, programs in scheduled runs. | Each a step of its own, started only when asked. |

## 6. Risks

- A new runtime dependency inside the worker. It needs an exact pin, a
  one-line justification and fuzzing, as `vak-ooxml` and `vak-pdf` have.
- An approval in the middle of a script holds the program; the Ask must be
  plain to the person, and a surface nobody watches must end it at once.
- A program hides steps from the person watching until P5 lands; P4
  should not reach everyday screens without it.
- If P0 finds few real fan-out turns, the saving may not pay for the
  surface. The baseline decides.

## 7. What it does not do

- No program reaches the network, the file system or a process except
  through a tool its session admits.
- No program decides whether work is complete; the runtime still does
  (invariant 33).
- No program sends anything outside Vak: effects stay one per reviewed
  candidate (invariant 41).
