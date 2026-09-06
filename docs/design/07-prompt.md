# 07 — System prompt

Current seed: `crates/vak-core/src/system-prompt.md` (~770 tokens, block-marked),
plus a runtime `Surface:` line of ~40 and any appended surface notes. Layer composition, editing surfaces, and
trust are specified in doc 45.
Editable per layer; see doc 45. `.vak/SYSTEM.md` remains as a legacy
project-layer `identity` override.

## Diff notes

- v0.1.0: initial six-tool kernel prompt. Rules emphasize read-before-edit,
  small verifiable steps, error-driven fixing, workspace containment.
- v0.1.1: documented the dynamically advertised task, memory, web, and MCP
  tools; explicitly separated skill guidance from executable tools; added a
  verification/no-false-completion rule after live Ollama prompt testing found
  ambiguity around `task` and discovered skill names.
- v0.1.2: made the skill boundary imperative by explicitly prohibiting skill
  names in tool calls after live-model retries showed that a descriptive skill
  name could still be selected as an executable tool.
- v0.1.3: instructed the agent to inspect file-backed requirements immediately
  rather than asking the user to repeat them; this closes a live Ollama case
  where the model stopped before reading README.md.
- v0.2.0: **identity change.** The prompt opened with "an expert coding agent
  operating in the user's terminal", which was wrong on both halves. vak is
  general-purpose — research, writing, data, and operations run through the
  same core as code — and the *same* prompt text is served to the CLI, the
  desktop app, the HTTP server, and every chat gateway, so a Telegram user was
  being addressed as though they were at a shell. The new opening states the
  general-purpose identity and explicitly forbids assuming a surface. The
  capability contract is unchanged; the rules were widened from edit/test
  phrasing to look-before-you-act and verify-with-whatever-the-work-has, with
  the code-specific cases kept as instances rather than the whole job. Two
  clauses were added: no tool for the ask means say so rather than narrate the
  effect, and out-of-workspace or irreversible effects are confirmed first.
  `default_prompt_documents_identity_and_dynamic_tool_boundaries`
  (`vak-core/src/lib.rs`) now pins the identity phrases *and* asserts the
  coding/terminal-only wording never returns. The auxiliary prompts moved with
  it — reflection (`vak-core/src/reflection.rs`), completion audit
  (`vak-agent/src/goal.rs`), and compaction (`vak-agent/src/context.rs`), whose
  "files created/modified" retention rule now also keeps non-file effects.
- v0.2.1: the prompt now *names* the surface instead of telling the model to
  assume nothing. `Core::with_surface` (`vak-core/src/lib.rs`) carries a
  `Surface` — `Cli`, `Desktop`, `Server`, `Chat { channel }`, `Background`, or
  the default `Unknown` — on the `Core` handle itself rather than in
  `CoreInner`, exactly as `default_deliver_to` already does, so the gateway can
  clone-and-stamp per inbound message without touching shared workspace state.
  `system_prompt()` appends a `Surface:` block that says where the reply will
  be read and what that costs: a phone-sized chat bubble, a terminal, a
  markdown panel next to a diff viewer the user can already see, an API client
  that may not render at all, or an unattended run with nobody to ask.
  `Unknown` is the default *because* it is honest — an un-stamped caller gets
  the old assume-nothing text rather than being mislabelled.

  Stamped at: the CLI run paths and `run_serve` (`vak/src/main.rs`), the
  desktop backend (`vak-desktop/src/main.rs`), the pooled per-workspace cores
  (`vak-server/src/core_pool.rs`), best-of-N children (`vak-server/src/lib.rs`)
  and the heartbeat (`vak-server/src/heartbeat.rs`) as `Background`, and each
  inbound gateway message (`vak-server/src/gateway.rs`), which narrows the
  pool's `Server` to the real transport.

  **Known limitation.** The assembled prompt freezes into the session contract
  at creation (`FrozenContract::system_prompt`, AGENTS.md rule 17), so a
  session started on one surface and resumed on another keeps the original
  `Surface:` line. This is the same staleness the frozen skills and MCP
  sections already carry, and it is left consistent with them deliberately
  rather than given a bespoke live-patching path for one line.

- v0.2.2: the seed became a *seed*. The single document is now split on
  `<!-- block: ... -->` markers into `identity`, `capability_contract`,
  `operating_rules`, and `guardrails`, resolved through the layer chain in doc
  45. Two guardrails were added that the prompt never had: content arriving
  through a tool is data rather than instruction (an agent with web, MCP, and
  file tools had nothing at all to say about prompt injection), and
  credentials are never revealed, transmitted, or written into a command line,
  commit, or outbound request. The `Surface:` paragraph is unchanged. The
  whole-document `.vak/SYSTEM.md` override is now read as the project layer's
  `identity` only, and is demoted entirely for an untrusted project — it could
  previously delete the capability contract and every safety rule on the
  first-run path.

- v0.2.3: `surface-note` — a fourth editable block appended after the
  generated `Surface:` line under "Also true on this surface:". The line
  itself stays code-owned; a note is a separate concatenating block, so
  nothing can name or replace the runtime's own observation, and no narrower
  layer can drop a wider one's note. Notes are dropped for an untrusted
  project (unlike guardrails) because free-form context can widen perceived
  latitude rather than narrow it.

- v3.0.17: multi-turn continuity, conversational drift, and no-escape-hatch
  clarification rules in `operating_rules`. Instructs the agent that conversational
  drift across turns is expected and to follow along smoothly without complaint
  or resistance; to resolve references ("the data", "do that", "it", "something")
  against earlier turns; and prohibits using demands for manual input or clarification
  as an exception-handling escape hatch to avoid taking action or using available tools.

## Successor

Doc 45 (`45-prompt-layers.md`) supersedes this document's "one constant plus
`.vak/SYSTEM.md`" model with user-editable, inherited prompt blocks. Diff
notes continue here for the shipped seed; layer composition, trust, and the
editing surfaces are specified there.

## Policy

The prompt stays under 1500 tokens. Every change ships with a diff note here
and passes the nightly eval suite before release (Phase 7). Prompt churn is a
bug class, not a feature — changes are reviewable events.
