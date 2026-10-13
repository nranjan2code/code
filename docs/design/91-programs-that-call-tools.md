# 91 — Programs that call tools

Status: proposal, 2026-10-13. Nothing here is built. The script language is
decided (§4.1, the maintainer, 2026-10-13); the other decisions in §4 are
open. The capability is the model writing one program that makes many
tool calls and returns only the answer. §2 lists the ways such programs go
wrong; the design in §3 is built to close each of them. Builds on doc 88
(the control plane), doc 89 (constrained tool arguments), doc 85 (the turn
graph) and doc 68 (the context engine). No step starts until the
maintainer says which stage to begin.

## 1. The gap

A fan-out job (find 40 files, read each, pull one field, compare them)
costs Vak one model step per call today, and every result passes through
the context on its way to the next step. The model reads 40 results it
needs one field of, and the turn's input grows with each step.

## 2. How programs that call tools go wrong

Running model-written code that calls tools is a known pattern, and its
failures are known too:

1. **One software layer between the code and the machine.** An
   interpreter that allow-lists built-ins, a language running on a bridge
   into its host runtime, an engine isolate inside the host process: each
   has been escaped by a single bug in that one layer, because nothing lay
   beneath it. Two recurring forms: an allowed built-in handed back
   unwrapped reaches a forbidden module, and a memory bug in an engine's
   native bindings becomes code execution in the host process.
2. **Host reach left on.** Loopback networking (code reaching services on
   the same machine), a compatibility layer of host APIs, a bridge object
   into the embedding runtime, inherited built-ins. None is needed to call
   tools, and each has been the way out.
3. **Presentation mistaken for permission.** A flag that says a tool may
   be called from code shapes what the model is shown; when nothing
   enforces it at the call, it is not a check.
4. **Results trusted as code or as clean data.** Tool output, especially
   from the web or mail, is written by whoever controls the source, and it
   flows straight into the next call's arguments.
5. **Data flow is invisible.** A program that reads private data and then
   calls anything that reaches the network can carry the data out inside an
   argument (a URL, a search query). Tracking where every value came from
   closes this, at a measurable cost in tasks completed.
6. **Fail-open reviewers and fixed timeouts.** A reviewer that allows a
   call when it cannot decide, and limits fixed in code where the operator
   cannot see them.
7. **Lossy results.** Results reduced to text, with images and binary
   content dropped, so a program silently works on less than the tool
   returned.

## 3. The shape

- One loop-owned tool, `program`, with one argument, `code`. The loop
  defines it, as it defines `plan`, `recall` and `task`, so it is
  recognised by name (AGENTS.md, "No word lists, no tuned limits").
- **Two layers, always** (closes 1). The code runs in a JavaScript engine
  embedded in the broker worker, as a new `WorkerTask::ProgramRun`, which
  moves the worker protocol (`PROTOCOL_VERSION` in
  `crates/vak-tools/src/broker.rs`) from 5 to 6. The worker runs under the
  OS sandbox read-only, network denied and rooted at the workspace,
  whatever the session's mode, as `VerifyTargets` and `OfficeReview` do
  (invariant 14). On Linux that is Landlock plus the seccomp filter that
  refuses IPv4 and IPv6 sockets and io_uring
  (`crates/vak-sandbox/src/landlock.rs`), so loopback is closed too; on
  macOS, Seatbelt's deny-default profile. An engine bug then reaches a
  process that can read the workspace and nothing else. A program never
  runs in the server, desktop or gateway process.
- **An empty world** (closes 2). The engine starts with the language's own
  built-ins and nothing from the host: no file, network, process, timer,
  clock, environment, module loader, evaluation of strings, or host
  compatibility layer. Randomness is seeded from the call's id. Its only
  way out is `await tools.<name>(args)`, a request on the worker's channel
  to the broker. Nothing is allow-listed out of a larger world, so there
  is no built-in left to forget.
- **Every inner call is a call** (closes 3). The broker handles it exactly
  as a call the model made: the one argument reader
  (`vak_tools::read_arguments`), the tool's `fill_defaults`, the schema
  check, `authorize` through `PermissionEngine` and the approver,
  execution in resource-claim waves, a child trace key
  (`TraceKey::child`) and a `Decision` entry for its verdict.
  `Tool::programmable` decides which tools are offered to programs, and the
  broker refuses an undeclared tool on every inner call, so the
  declaration is a check, not a hint. A program never reaches more than
  the model could by calling the same tools one by one.
- **Results are data with a declared shape** (closes 4 and 7). A
  programmable tool declares the shape of its result
  (`Tool::output_schema`), and the script receives parsed JSON of that
  shape, never text to interpret; nothing in the engine evaluates a
  string. A binary result arrives as a tenant object reference that the
  script can pass on to another tool, never dropped.
- **Provenance travels with the program** (closes 5). Each inner result
  carries the label its source already has in Vak: the conversation's own,
  a connected account's (`ToolOutput::from_account`), an intake item's
  disposition, or the open web. Vak tracks it per program, not per value:
  once a program has received a result labelled private (an account's
  data, a held item, a secret's file) or untrusted (the web, an intake
  item, an MCP result), any later inner call whose declared reach leaves
  the machine asks the approver, whatever the mode, and an unwatched
  surface refuses it. Private or untrusted data then cannot leave by an
  argument without a person, at a cost only to programs that mix the two.
  Per-value labels inside the engine are a later option (§6).
- **No fail-open and no fixed timeouts** (closes 6). An Ask that cannot
  reach an approver is a denial returned to the script. Wall time and
  memory are the operator's limits (§3.1).
- An inner call's error comes back to the script as a value carrying its
  `vak_control::Fault`, so the script can handle it. A program that fails
  is `ToolOutput::error(Fault::…)`, naming where the fault lies.
- **Replayable.** The script's only inputs are its code and its inner
  results, all in the ledger, and the engine has no clock or unseeded
  randomness, so running the code again against the recorded results
  reproduces its output. An audit can check that a program's answer is
  what its code makes of what its tools returned.
- Only what the script returns is model-visible. Every inner call is in
  the ledger and reachable by `recall`.

### 3.1 Invariants it keeps

| Invariant | How |
|---|---|
| 1, model-visible means logged | The `program` call (its code untruncated in the arguments) and its result are ordinary entries. Inner calls are a new audit-only entry, `EntryPayload::ProgramCall`, naming the outer `tool_use_id`, the turn (`Entry::at_turn`) and the result's provenance label; `derive_messages()` never reads it. |
| 2, append-only | Inner calls are appended as they finish; nothing is rewritten when the program ends. |
| 5, abort keeps partial output | Cancellation threads into the worker and stops the engine; inner calls already made stay recorded and recallable. |
| 10, workspace-rooted access | The worker's sandbox and every inner file tool resolve inside the session workspace. |
| 12, secrets are not ambient | The worker gets no credentials; a tool that needs one gets it from the broker as it does today. |
| 14, broker boundary | The worker gets the code and a call channel, never the policy engine, approvals, credentials or session store. |
| 15, unattended surfaces fail closed | An Ask inside a program on a surface nobody answers is a denial, returned to the script as a value. |
| 16, authorize before dispatch | Inner calls go through `authorize`; an Ask blocks the script until the approver answers. |
| 36, nothing cut blind | The returned value is handled as any long tool result is; no character truncation. Inner results stay out of the working set and are reached by `recall`. |
| 40, trace key | Each inner call carries a child of the `program` call's key; the catalog traces it to the run. |
| Declared reach | A tool joins programs only by a declaration, `Tool::programmable` (default false), forwarded by every wrapper; never a list of names. Card tools, `plan`, `task`, `work` and anything that acts outside Vak (an effect) never declare it. |
| No tuned limits | Wall time and the engine's memory are operator limits in `[control.limits]` (`TimeLimits::program_secs` and a memory ceiling beside it, each shown in Settings with its shipped default), and `program` names its time argument through `Tool::time_limit_argument`. There is no cap on inner calls; the operator's turn limit still bounds the run. |

## 4. Decisions

1. **Script language: decided.** JavaScript, in an engine embedded in the
   isolated worker, never the machine's interpreter and never a language
   of Vak's own. The point is isolation: programs run at the level of an
   isolated worker, not at the level of the machine (§2.1 and §2.2).
2. **Which engine.** Open. The criteria, in order: memory safety (the
   worst escapes in §2.1 were memory bugs in native bindings), no JIT
   (less native surface and a deterministic replay), enough of the
   language that ordinary model-written code runs (async functions,
   `Promise.all`, JSON, destructuring), then size and start time. P0
   measures the candidate engines against these on programs models wrote
   for the P0 tasks, and P2 pins the chosen one exactly in the workspace
   manifest with its one-line justification.
3. **First reach.** Recommended: only tools whose declared reach reads
   (`read`, `grep`, `glob`, `doc_read`, `session_search`, `webfetch`, and
   MCP tools that declare it), with the provenance rule governing
   `webfetch` after a private or untrusted result. Writes follow in P6,
   each a draft, so Review still decides what reaches the workspace.
4. **How the model learns the bindings.** Recommended: a typed signature
   per programmable tool, derived from its `call_schema` and
   `output_schema`, served through `find_tools`, not added to the
   byte-stable prefix, so the prompt seed's budget and cache stay as they
   are.
5. **Long code as one argument.** Doc 89 measured that long string
   arguments are the ones that break. Recommended: `program` is a strict
   tool where the wire enforces a schema, the regeneration path covers the
   rest, and the form is measured with `scripts/measure_tool_form.py` on
   Vak's captured requests before P4 ships.

## 5. Stages

Each stage ships whole with its tests and docs, and replaces nothing it
leaves behind (invariant 30).

| Stage | Work | Done when |
|---|---|---|
| P0 · Baseline | Capture real fan-out turns in a `/tmp` workspace on at least two routes; record steps, input tokens and wall time per task in `docs/audits/`. Measure the candidate engines (§4.2) on programs models wrote for those tasks. | The baseline and the engine measurements are committed and §4 records the remaining decisions. |
| P1 · Declarations and ledger | `Tool::programmable` and `Tool::output_schema` (forwarded by every wrapper); `EntryPayload::ProgramCall` (traced, `at_turn`, parent call id, provenance label, a large result as a tenant object); a catalog source for it; `docs/reference/records.md` regenerated. | `derive_messages()` is byte-identical with and without inner entries; `every_ledger_row_type_is_traced` passes. |
| P2 · Worker runtime | The pinned engine behind `WorkerTask::ProgramRun` with an empty world, the call channel, cancellation, the operator's deadline and memory ceiling, seeded randomness; protocol 6. | An escape corpus fails closed on macOS and Linux: every failure in §2 restated as a test (host built-ins, bridges, loopback and Unix-socket connects, prototype and global tricks, string evaluation), plus endless loops, deep recursion and huge returns; a fuzz target runs in CI; replaying a recorded program reproduces its output. |
| P3 · Broker dispatch | Inner calls through reader, `fill_defaults`, `authorize`, execution in waves, approvals, child trace keys, `Decision` entries, typed faults returned to the script; the broker's refusal of undeclared tools; the provenance rule. | Tests in every permission mode, an unattended channel included, show a program cannot call what its session could not, and that a program which read an account's data or a web page cannot reach the network without the approver. |
| P4 · The loop tool | Register `program`; its result enters context, inner results stay out and are reachable by `recall`; the plan check (`vak_judge::plan`) reads inner receipts; workers spawned by `task` get it too. | The P0 tasks run live on two routes with fewer steps and fewer input tokens than the baseline, and no result lost. |
| P5 · Surfaces | One call in the transcript that opens to its inner calls with their provenance; Workbench shows them live; the admin Run detail shows them in the span tree. Both client bundles rebuilt. | Checked in a browser at 1440 × 900 and 390 × 844, light and dark. |
| P6 · Later | Write reach (each write a draft for Review), MCP tools by their own declaration, programs in scheduled runs, per-value provenance in the engine. | Each a step of its own, started only when asked. |

## 6. Risks

- The engine is new native surface in the worker. It needs an exact pin,
  a one-line justification and fuzzing, as `vak-ooxml` and `vak-pdf` have,
  and the OS sandbox stays beneath it however good it is.
- The provenance rule is per program, so a program that reads one web
  page and then searches will ask. If P4 shows that this asks too often,
  the answer is per-value labels in the engine, not a looser rule.
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
