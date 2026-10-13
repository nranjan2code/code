# 91 — Programs that call tools

Status: proposal, 2026-10-13. Nothing here is built. The script language is
decided (§4.1, the maintainer, 2026-10-13); the other decisions in §4 are
open. Written at the maintainer's request after a design comparison with
Nous Research's Hermes Agent (read from its source at `6e7b83a2`) found one
capability Vak has no answer to: the model writing one program that makes
many tool calls and returns only the answer. §2 surveys how eight other
systems do it and where each failed; the design in §3 is built from those
failures. Builds on doc 88 (the control plane), doc 89 (constrained tool
arguments), doc 85 (the turn graph) and doc 68 (the context engine). No
step starts until the maintainer says which stage to begin.

## 1. The gap

A fan-out job (find 40 files, read each, pull one field, compare them)
costs Vak one model step per call today, and every result passes through
the context on its way to the next step. The model reads 40 results it
needs one field of, and the turn's input grows with each step.

## 2. What the field does, and where it failed

| System | Language and runtime | Where the code runs | Recorded failure or stated limit |
|---|---|---|---|
| Hermes `execute_code` | Python; a generated stub module calls tools over a Unix socket | A host subprocess, outside Hermes' own terminal backend (its SECURITY.md §2.2) | Unconfined by design: its policy says only a whole-process wrapper confines it. |
| Anthropic programmatic tool calling | Python in Anthropic's code-execution container; the call pauses and the client returns each result | Anthropic's container | `allowed_callers` "is not a hard API-level block"; strict tools are not supported with it; a pending call times out after about four minutes; MCP-connector tools cannot be called; tool results are strings and "may contain code" to be validated. |
| Cloudflare Code Mode | TypeScript against a generated SDK, in a workerd V8 isolate; two tools, `search` and `execute` | An isolate inside one process | Check Point (2026) found five memory bugs in workerd's C++ glue (URLPattern, `node:zlib`, HTMLRewriter, SQL authorizer); a prompt injection became host code execution where the V8 cage was off. Node compatibility on by default widened the reachable native surface. |
| Goose Code Mode | JavaScript via `pctx_code_mode` on a vendored V8, over MCP tools | Inside the Goose process | Text-only results (images and binary dropped). Goose's default permission mode is fully autonomous, and its adversary reviewer is an LLM that fails open. |
| smolagents `CodeAgent` | Python, interpreted by an allow-listing AST evaluator | In the host process | CVE-2025-5120 (CVSS 9.9): an allowed builtin (`getattr`) returned unwrapped reached `sys` through an allowed module; the safety check judged only an expression's final value. |
| Pydantic `mcp-run-python` | Python on Pyodide inside Deno | A Deno subprocess | CVE-2026-25904: the sandbox could reach the host's loopback (SSRF into local services); a second advisory: Python reached the JavaScript bridge (`js`) and escaped. Archived; replaced by Monty, a Python subset written in Rust with no file system, environment or network. |
| TanStack AI Code Mode | TypeScript via `isolated-vm` (V8) or QuickJS compiled to WebAssembly | In process, or a Worker | A fresh isolate per run; memory, stack and wall limits are library defaults; no published threat model. |
| CaMeL (Google DeepMind) and FIDES (Microsoft) | The planner model writes code; a custom interpreter runs it | Research systems | Values carry provenance and permission labels, and a policy checks them before each tool call, so untrusted data never decides control flow. CaMeL solved 67% of AgentDojo tasks with that guarantee: security costs completion. |

What repeats:

1. **One software layer between model-written code and the host.** Every
   escape above broke the only boundary there was: an AST allow-list, a
   language bridge, a C++ binding beneath an isolate. None had an OS
   boundary underneath.
2. **Host reach left on.** Loopback, Node compatibility, a JavaScript
   bridge, inherited builtins: each was a convenience nobody needed for
   calling tools.
3. **Presentation mistaken for permission.** A flag that says a tool may
   be called from code shapes what the model sees; it is not a check.
4. **Results trusted as code or as clean data.** Tool output, especially
   from the web or mail, is attacker-influenced and flows straight into the
   next call's arguments.
5. **Data flow is invisible.** A program that reads private data and then
   calls anything that reaches the network can carry the data out in an
   argument (a URL, a search query). Only CaMeL and FIDES address it, and
   they pay in completion.
6. **Fail-open reviewers and fixed timeouts.** A reviewer that allows on
   error, and limits hardcoded in the vendor's code.

## 3. The shape

- One loop-owned tool, `program`, with one argument, `code`. The loop
  defines it, as it defines `plan`, `recall` and `task`, so it is
  recognised by name (AGENTS.md, "No word lists, no tuned limits").
- **Two layers, always** (answers failure 1). The code runs in a
  JavaScript engine embedded in the broker worker, as a new
  `WorkerTask::ProgramRun`, which moves the worker protocol
  (`PROTOCOL_VERSION` in `crates/vak-tools/src/broker.rs`) from 5 to 6.
  The worker runs under the OS sandbox read-only, network denied and
  rooted at the workspace, whatever the session's mode, as `VerifyTargets`
  and `OfficeReview` do (invariant 14). On Linux that is Landlock plus the
  seccomp filter that refuses IPv4 and IPv6 sockets and io_uring
  (`crates/vak-sandbox/src/landlock.rs`), so loopback is closed too
  (answers the `mcp-run-python` SSRF); on macOS, Seatbelt's deny-default
  profile. An engine bug then reaches a process that can read the
  workspace and nothing else. The program never runs in the server,
  desktop or gateway process.
- **An empty world** (answers failure 2). The engine starts with the
  language's own built-ins and nothing from the host: no file, network,
  process, timer, clock, environment, module loader, `eval` of strings or
  Node compatibility. Randomness is seeded from the call's id. Its only
  way out is `await tools.<name>(args)`, a request on the worker's channel
  to the broker. Nothing is allow-listed out of a larger world, so there
  is no `getattr` to forget.
- **Every inner call is a call** (answers failure 3). The broker handles
  it exactly as a call the model made: the one argument reader
  (`vak_tools::read_arguments`), the tool's `fill_defaults`, the schema
  check, `authorize` through `PermissionEngine` and the approver,
  execution in resource-claim waves, a child trace key
  (`TraceKey::child`) and a `Decision` entry for its verdict.
  `Tool::programmable` decides which tools are offered to programs, and the
  broker refuses an undeclared tool on every inner call, so the
  declaration is a check, not a hint. A program never reaches more than
  the model could by calling the same tools one by one.
- **Results are data with a declared shape** (answers failure 4). A
  programmable tool declares the shape of its result (`Tool::output_schema`),
  and the script receives parsed JSON of that shape, never text to
  interpret; nothing in the engine evaluates a string. A binary result
  arrives as a tenant object reference, not as dropped content.
- **Provenance travels with the program** (answers failure 5). Each inner
  result carries the label its source already has in Vak: the
  conversation's own, a connected account's (`ToolOutput::from_account`),
  an intake item's disposition, or the open web. Vak tracks it per
  program, not per value: once a program has received a result labelled
  private (an account's data, a held item, a secret's file) or untrusted
  (the web, an intake item, an MCP result), any later inner call whose
  declared reach leaves the machine asks the approver, whatever the mode,
  and an unwatched surface refuses it. This keeps CaMeL's property for
  the case that matters (private or untrusted data cannot leave by an
  argument without a person) without its interpreter, at a cost only to
  programs that mix the two. Per-value labels inside the engine are a
  later option (§6).
- **No fail-open and no fixed timeouts** (answers failure 6). An Ask that
  cannot reach an approver is a denial returned to the script. Wall time
  is the operator's limit; see §3.1.
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
   isolated worker, not at the level of the machine. Every failure in §2
   that reached a host went through a runtime that sat directly on it.
2. **Which engine.** Open. The criteria, from §2: memory safety first
   (the Code Mode escapes were use-after-free bugs in native glue), then
   no JIT (less native surface and a deterministic replay), then enough of
   the language that models' ordinary code runs (async functions,
   `Promise.all`, JSON, destructuring), then size and start time.
   Recommended: Boa (`boa_engine`), written in Rust with no JIT; the
   alternative is QuickJS through `rquickjs` (C, smaller and more
   complete, not memory-safe); V8 (`deno_core`) is rejected for its size
   and JIT surface. P0 measures the candidates on captured programs before
   P2 pins one exactly in the workspace manifest.
3. **First reach.** Recommended: only tools whose declared reach reads
   (`read`, `grep`, `glob`, `doc_read`, `session_search`, `webfetch`, and
   MCP tools that declare it), with the provenance rule above governing
   `webfetch` after a private or untrusted result. Writes follow in P6,
   each a draft, so Review still decides what reaches the workspace.
4. **How the model learns the bindings.** Recommended: a typed signature
   per programmable tool, derived from its `call_schema` and
   `output_schema`, served through `find_tools`, not added to the
   byte-stable prefix, so the prompt seed's budget and cache stay as they
   are.
5. **Long code as one argument.** Doc 89 measured that long string
   arguments are the ones that break. Recommended: `program` is a strict
   tool where the wire enforces a schema (it is Vak's own tool, so the
   Anthropic restriction on strict tools in their container does not
   apply), the regeneration path covers the rest, and the form is measured
   with `scripts/measure_tool_form.py` on Vak's captured requests before
   P4 ships.

## 5. Stages

Each stage ships whole with its tests and docs, and replaces nothing it
leaves behind (invariant 30).

| Stage | Work | Done when |
|---|---|---|
| P0 · Baseline | Capture real fan-out turns in a `/tmp` workspace on at least two routes; record steps, input tokens and wall time per task in `docs/audits/`. Run the candidate engines (§4.2) on programs models wrote for those tasks. | The baseline and the engine measurements are committed and §4 records the remaining decisions. |
| P1 · Declarations and ledger | `Tool::programmable` and `Tool::output_schema` (forwarded by every wrapper); `EntryPayload::ProgramCall` (traced, `at_turn`, parent call id, provenance label, a large result as a tenant object); a catalog source for it; `docs/reference/records.md` regenerated. | `derive_messages()` is byte-identical with and without inner entries; `every_ledger_row_type_is_traced` passes. |
| P2 · Worker runtime | The pinned engine behind `WorkerTask::ProgramRun` with an empty world, the call channel, cancellation, the operator's deadline and memory ceiling, seeded randomness; protocol 6. | An escape corpus fails closed on macOS and Linux: every §2 failure restated as a test (host built-ins, bridges, loopback and Unix-socket connects, prototype and global tricks, string evaluation), plus endless loops, deep recursion and huge returns; a fuzz target runs in CI; replaying a recorded program reproduces its output. |
| P3 · Broker dispatch | Inner calls through reader, `fill_defaults`, `authorize`, execution in waves, approvals, child trace keys, `Decision` entries, typed faults returned to the script; the broker's refusal of undeclared tools; the provenance rule. | Tests in every permission mode, an unattended channel included, show a program cannot call what its session could not, and that a program which read an account's data or a web page cannot reach the network without the approver. |
| P4 · The loop tool | Register `program`; its result enters context, inner results stay out and are reachable by `recall`; the plan check (`vak_judge::plan`) reads inner receipts; workers spawned by `task` get it too. | The P0 tasks run live on two routes with fewer steps and fewer input tokens than the baseline, and no result lost. |
| P5 · Surfaces | One call in the transcript that opens to its inner calls with their provenance; Workbench shows them live; the admin Run detail shows them in the span tree. Both client bundles rebuilt. | Checked in a browser at 1440 × 900 and 390 × 844, light and dark. |
| P6 · Later | Write reach (each write a draft for Review), MCP tools by their own declaration, programs in scheduled runs, per-value provenance in the engine. | Each a step of its own, started only when asked. |

## 6. Risks

- The engine is new native surface in the worker. It needs an exact pin,
  a one-line justification, and fuzzing, as `vak-ooxml` and `vak-pdf`
  have, and the OS sandbox stays beneath it however good it is.
- The provenance rule is per program, so a program that reads one web
  page and then searches will ask. If P4 shows that this asks too often,
  the answer is per-value labels in the engine (CaMeL's design), not a
  looser rule.
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

## Sources

- Hermes Agent, `website/docs/user-guide/features/code-execution.md` and
  `SECURITY.md`, at `6e7b83a2`.
- Anthropic, "Programmatic tool calling" (platform.claude.com docs) and
  "Code execution with MCP" (anthropic.com/engineering).
- Check Point Research, "When agentic glue melts" (2026), on workerd.
- Goose, `documentation/docs/guides/managing-tools/code-mode.md`,
  `goose-permissions.md` and `security/adversary-mode.md`.
- GHSA-6v92-r5mx-h5fx / CVE-2025-5120 (smolagents); CVE-2026-25904
  (`mcp-run-python`).
- TanStack AI, "Code Mode isolate drivers".
- Debenedetti et al., "Defeating Prompt Injections by Design" (CaMeL);
  Costa et al., "Securing AI Agents with Information-Flow Control"
  (FIDES, arXiv 2505.23643).
