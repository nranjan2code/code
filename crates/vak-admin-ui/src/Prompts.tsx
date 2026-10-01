import { For, Show, createEffect, createMemo, createResource, createSignal } from "solid-js";

import { api } from "./api";
import { selectedAgentId, selectedAgentIdOrUndefined } from "./store";
import type {
  ConfigScope,
  PromptBlock,
  PromptEffective,
  PromptLayerDescriptor,
} from "./types";

/// Prompt layers (docs/design/45-prompt-layers.md).
///
/// Composition runs through seven tiers (seed -> shared -> project -> surface ->
/// bot -> chat -> agent role). Doc 44 requires every editable surface to report
/// the selected layer *and* the effective result with provenance.

interface BlockDef {
  id: PromptBlock;
  label: string;
  dotClass: string;
  help: string;
}

const BLOCKS: BlockDef[] = [
  {
    id: "identity",
    label: "Identity",
    dotClass: "dot-identity",
    help: "Who the agent is. The narrowest setting that defines this wins.",
  },
  {
    id: "operating-rules",
    label: "Operating rules",
    dotClass: "dot-rules",
    help: "How it works. The narrowest setting that defines this wins.",
  },
  {
    id: "guardrails",
    label: "Guardrails",
    dotClass: "dot-guardrails",
    help: "Guardrails from every scope apply together. A narrower scope cannot remove one.",
  },
  {
    id: "surface-note",
    label: "Surface note",
    dotClass: "dot-surface",
    help: "Appended after the generated Surface line — what this deployment knows about where the reply lands. Accumulates across scopes.",
  },
];

const LAYER_LABELS: Record<PromptLayerDescriptor["layer"], string> = {
  seed: "Shipped default",
  shared: "Global",
  project: "Workspace",
  surface: "Surface",
  bot: "Bot",
  chat: "Chat",
  agent: "Agent role",
};

const LAYER_CSS_CLASS: Record<PromptLayerDescriptor["layer"], string> = {
  seed: "seed",
  shared: "shared",
  project: "project",
  surface: "surface",
  bot: "bot",
  chat: "chat",
  agent: "agent",
};

const SURFACES = [
  { id: "cli", label: "CLI" },
  { id: "desktop", label: "Desktop" },
  { id: "server", label: "Server" },
  { id: "background", label: "Background" },
  { id: "worker", label: "Worker" },
  { id: "telegram", label: "Telegram" },
] as const;

/// Universal Agent templates (Research, Writing, Data, Operations, Life/Work Automation, Engineering)
const UNIVERSAL_TEMPLATES: Record<PromptBlock, { label: string; snippet: string }[]> = {
  "identity": [
    {
      label: "Universal Agent",
      snippet: "You are vak, a general-purpose agent working on the user's behalf. You do real work, not just talk about it: answering questions, researching, writing, analysing data, building software, running commands, managing files, fetching information, drafting documents, and operating systems. Engineering, research, writing, data, operations, and ordinary questions are all equally your work. Read each request for what it actually asks and do that; never reshape it into a different kind of task because that kind is more familiar.",
    },
    {
      label: "Research & Synthesis",
      snippet: "You are an analytical researcher and synthesizer. Formulate evidence-backed arguments, cross-examine claims, cite verifiable sources, and explicitly identify nuances, assumptions, and edge cases.",
    },
    {
      label: "Executive Writing",
      snippet: "You are an executive writer and communications specialist. Craft clear, high-impact prose with crisp executive summaries, structured headers, and active voice.",
    },
    {
      label: "Data & Insights",
      snippet: "You are a quantitative data analyst. Uncover statistical patterns, highlight anomalies, structure insights into comparative tables, and quantify decision impacts.",
    },
    {
      label: "Systems & Ops",
      snippet: "You are a systems operations specialist. Prioritize site reliability, non-destructive validation, clear runbooks, and robust telemetry.",
    },
  ],
  "operating-rules": [
    {
      label: "Outcome-directed prose",
      snippet: "- Read each request for what it actually asks and do that; never reshape it into a different kind of task because that kind is more familiar.",
    },
    {
      label: "Grounded verification",
      snippet: "- Always verify state before making assertions or edits. Present concrete facts and evidence rather than assumptions.",
    },
    {
      label: "Structured and scannable",
      snippet: "- Keep prose structured, dense, and scannable. Use tables and bulleted lists where appropriate.",
    },
  ],
  "guardrails": [
    {
      label: "Confirm destructive actions",
      snippet: "- Always ask for human confirmation before deleting files, dropping tables, or executing irreversible system commands",
    },
    {
      label: "Protect privacy and secrets",
      snippet: "- Never print, log, or leak API keys, auth tokens, passwords, or personal identifying information",
    },
    {
      label: "Ground in verified sources",
      snippet: "- Ground assertions in verified sources. Distinguish observed data from model inference",
    },
    {
      label: "Structured output format",
      snippet: "- Return structured output as requested without conversational preamble or pleasantries",
    },
  ],
  "surface-note": [
    {
      label: "Public channel caution",
      snippet: "- This is a shared channel; assume anyone can inspect the conversation",
    },
    {
      label: "Background automation",
      snippet: "- Running unattended: prioritize non-blocking, idempotent operations",
    },
    {
      label: "Read-only observation",
      snippet: "- Perform inspection and reporting only: do not perform any state-modifying actions",
    },
  ],
};

interface PromptHistoryEntry {
  id: string;
  timestamp: string;
  block: PromptBlock;
  scope: ConfigScope;
  text: string;
  chars: number;
}

function loadHistory(): PromptHistoryEntry[] {
  try {
    const raw = localStorage.getItem("vak_prompt_history");
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function saveHistoryEntry(entry: PromptHistoryEntry) {
  try {
    const current = loadHistory();
    const next = [entry, ...current.slice(0, 19)];
    localStorage.setItem("vak_prompt_history", JSON.stringify(next));
  } catch {
    // ignore local storage errors
  }
}

function scopeLabel(scope: ConfigScope): string {
  return scope === "user" ? "Global" : "Workspace";
}

export function PromptsSection(props: {
  scope: () => ConfigScope;
  pushToast: (kind: "info" | "warn" | "alert", text: string) => void;
  onScopeChange?: (scope: ConfigScope) => void;
}) {
  const [layer, { refetch: refetchLayer }] = createResource(
    () => ({ scope: props.scope(), agent: selectedAgentIdOrUndefined() }),
    ({ scope, agent }) => api.promptLayer(scope, agent),
  );
  const [effective, { refetch: refetchEffective }] = createResource<PromptEffective, string | undefined>(
    selectedAgentIdOrUndefined,
    (agent) => api.promptEffective(agent),
  );
  const [roles, { refetch: refetchRoles }] = createResource(selectedAgentId, () => api.promptRoles(selectedAgentIdOrUndefined()));

  const [editing, setEditing] = createSignal<PromptBlock | null>(null);
  const [draft, setDraft] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [previewSurface, setPreviewSurface] = createSignal<string>("cli");
  const [previewRole, setPreviewRole] = createSignal("");
  const [preview, setPreview] = createSignal<PromptEffective | null>(null);

  // Filter for prompt blocks
  const [blockFilter, setBlockFilter] = createSignal<"all" | PromptBlock | "code-owned">("all");
  // Search query within preview
  const [searchQuery, setSearchQuery] = createSignal("");
  // Copy state
  const [copiedFingerprint, setCopiedFingerprint] = createSignal(false);
  const [copiedPrompt, setCopiedPrompt] = createSignal(false);
  // History list
  const [history, setHistory] = createSignal<PromptHistoryEntry[]>(loadHistory());
  const [showHistory, setShowHistory] = createSignal(false);
  // Selected table block
  const [selectedTableBlock, setSelectedTableBlock] = createSignal<string | null>(null);
  // Preview view mode
  const [previewViewMode, setPreviewViewMode] = createSignal<"full" | "sections">("full");
  let promptPreRef: HTMLPreElement | undefined;

  function scrollToSurface() {
    if (previewViewMode() !== "full") {
      setSelectedTableBlock("surface");
      return;
    }
    setTimeout(() => {
      if (promptPreRef) {
        promptPreRef.scrollTop = promptPreRef.scrollHeight;
      }
    }, 40);
  }

  const blockText = (block: PromptBlock): string | null => {
    const l = layer()?.layer;
    if (!l) return null;
    if (block === "identity") return l.identity ?? null;
    if (block === "operating-rules") return l.operating_rules ?? null;
    const rules = (block === "guardrails" ? l.guardrails : l.surface_notes) ?? [];
    return rules.length ? rules.map((r) => `- ${r}`).join("\n") : null;
  };

  const inheritedText = (block: PromptBlock): string | null => {
    const eff = preview() ?? effective();
    if (!eff) return null;
    return eff.blocks?.[block] ?? eff.seed_blocks?.[block] ?? null;
  };

  const contributors = (block: PromptBlock): PromptLayerDescriptor[] =>
    ((preview() ?? effective())?.layers ?? []).filter((d) => d.block === block);

  async function save(block: PromptBlock) {
    setBusy(true);
    const content = draft();
    try {
      await api.putPromptBlock(props.scope(), block, content, selectedAgentIdOrUndefined());
      props.pushToast(
        "info",
        `${block} saved to ${scopeLabel(props.scope())}. Applies from the next turn.`,
      );
      saveHistoryEntry({
        id: Date.now().toString(),
        timestamp: new Date().toLocaleTimeString(),
        block,
        scope: props.scope(),
        text: content,
        chars: content.length,
      });
      setHistory(loadHistory());
      setEditing(null);
      await Promise.all([refetchLayer(), refetchEffective()]);
    } catch (e) {
      props.pushToast("alert", `Could not save: ${(e as Error).message}`);
    } finally {
      setBusy(false);
    }
  }

  async function reset(block: PromptBlock) {
    setBusy(true);
    try {
      await api.putPromptBlock(props.scope(), block, null, selectedAgentIdOrUndefined());
      props.pushToast("info", `${block} reset; it is inherited again.`);
      setEditing(null);
      await Promise.all([refetchLayer(), refetchEffective()]);
    } catch (e) {
      props.pushToast("alert", `Could not reset: ${(e as Error).message}`);
    } finally {
      setBusy(false);
    }
  }

  async function runPreview(surface = previewSurface(), role = previewRole()) {
    try {
      const res = await api.promptPreview(surface, role || undefined, selectedAgentIdOrUndefined());
      setPreview(res);
    } catch (e) {
      props.pushToast("alert", `Preview failed: ${(e as Error).message}`);
    }
  }

  createEffect(() => {
    const s = previewSurface();
    const r = previewRole();
    void runPreview(s, r);
  });

  const tokens = () => preview()?.estimated_tokens ?? effective()?.estimated_tokens ?? 0;
  const overBudget = () => tokens() > 1800;
  const currentFingerprint = () => preview()?.fingerprint ?? effective()?.fingerprint ?? "";

  const renderedText = () => preview()?.text ?? effective()?.text ?? "";

  // Dynamic extraction of what the chosen surface and role inject
  const surfaceLine = createMemo(() => {
    const text = renderedText();
    const lines = text.split("\n");
    const found = lines.find((l) => l.startsWith("Surface:"));
    return found ?? `Surface: ${previewSurface()}`;
  });

  const roleInstruction = createMemo(() => {
    const text = renderedText();
    const idx = text.indexOf("Agent-specific instructions");
    if (idx === -1) return null;
    const slice = text.slice(idx);
    const end = slice.indexOf("\n\n");
    const section = end !== -1 ? slice.slice(0, end) : slice;
    const lines = section.split("\n").filter((l) => l.startsWith("- "));
    return lines.length ? lines.join("\n") : null;
  });

  const searchMatches = createMemo(() => {
    const q = searchQuery().trim().toLowerCase();
    if (!q) return [];
    const lines = renderedText().split("\n");
    return lines
      .map((line, idx) => ({ line, num: idx + 1 }))
      .filter((item) => item.line.toLowerCase().includes(q));
  });

  const totalBytes = createMemo(() => {
    const layers = (preview() ?? effective())?.layers ?? [];
    return layers.reduce((sum, d) => sum + d.bytes, 0) || 1;
  });

  const activeLayersCount = createMemo(() => {
    const layers = (preview() ?? effective())?.layers ?? [];
    const unique = new Set(layers.map((l) => l.layer));
    return unique.size;
  });

  function insertSnippet(snippet: string) {
    const cur = draft().trim();
    if (!cur) {
      setDraft(snippet);
    } else {
      setDraft(cur + "\n" + snippet);
    }
  }

  function startOverride(block: PromptBlock) {
    const existing = blockText(block);
    const baseline = inheritedText(block);
    setDraft(existing ?? baseline ?? "");
    setEditing(block);
  }

  async function copyToClipboard(text: string, kind: "fingerprint" | "prompt") {
    try {
      await navigator.clipboard.writeText(text);
      if (kind === "fingerprint") {
        setCopiedFingerprint(true);
        setTimeout(() => setCopiedFingerprint(false), 2000);
        props.pushToast("info", "Prompt fingerprint copied to clipboard");
      } else {
        setCopiedPrompt(true);
        setTimeout(() => setCopiedPrompt(false), 2000);
        props.pushToast("info", "Full assembled prompt copied to clipboard");
      }
    } catch {
      props.pushToast("warn", "Failed to copy to clipboard");
    }
  }

  return (
    <section class="prompts-section">
      {/* ---------------------------------- Executive Telemetry Deck -- */}
      <div class="prompt-deck">
        <div class={`prompt-kpi-card ${overBudget() ? "warn" : ""}`}>
          <div class="prompt-kpi-head">
            <span>Context Budget</span>
            <span class={`chip ${overBudget() ? "chip-warn" : "chip-ok"}`}>
              {overBudget() ? "Standing Cost" : "Optimal"}
            </span>
          </div>
          <div class="prompt-kpi-val">~{tokens().toLocaleString()} tokens</div>
          <div class="prompt-budget-bar">
            <div
              class={`prompt-budget-fill ${overBudget() ? "warn" : ""}`}
              style={{ width: `${Math.min(100, Math.round((tokens() / 4000) * 100))}%` }}
            />
          </div>
          <div class="prompt-kpi-detail">
            {overBudget()
              ? "Over 1,500 threshold — standing per-turn overhead"
              : "Within lean context budget recommendations"}
          </div>
        </div>

        <div class="prompt-kpi-card">
          <div class="prompt-kpi-head">
            <span>Editing Scope</span>
            <Show when={props.onScopeChange}>
              <div class="prompt-scope-toggle">
                <button
                  class="prompt-scope-btn"
                  classList={{ active: props.scope() === "user" }}
                  onClick={() => props.onScopeChange?.("user")}
                >
                  Global
                </button>
                <button
                  class="prompt-scope-btn"
                  classList={{ active: props.scope() === "project" }}
                  onClick={() => props.onScopeChange?.("project")}
                >
                  Workspace
                </button>
              </div>
            </Show>
          </div>
          <div class="prompt-kpi-val">{scopeLabel(props.scope())}</div>
          <div class="prompt-kpi-detail" title={layer()?.path ?? ""}>
            <code>{layer()?.path ? layer()!.path.split("/").slice(-2).join("/") : "loading…"}</code>
          </div>
        </div>

        <div class="prompt-kpi-card">
          <div class="prompt-kpi-head">
            <span>Layer Composition</span>
            <span class="chip chip-kind">{activeLayersCount()} tiers active</span>
          </div>
          <div class="prompt-kpi-val">{(preview() ?? effective())?.layers.length ?? 0} slices</div>
          <div class="prompt-kpi-detail">
            {(preview() ?? effective())?.layers.map((l) => LAYER_LABELS[l.layer]).slice(0, 3).join(" · ") || "Seed only"}
          </div>
        </div>

        <div class="prompt-kpi-card">
          <div class="prompt-kpi-head">
            <span>Fingerprint</span>
            <button
              class="ghost small"
              style="padding: 1px 6px; font-size: 12px;"
              onClick={() => copyToClipboard(currentFingerprint(), "fingerprint")}
            >
              {copiedFingerprint() ? "Copied" : "Copy hash"}
            </button>
          </div>
          <div class="prompt-kpi-val" style="font-size: 15px;">
            {currentFingerprint() ? currentFingerprint().slice(0, 14) : "—"}
          </div>
          <div class="prompt-kpi-detail">
            SHA-256 receipt for prompt audit verification
          </div>
        </div>
      </div>

      <Show when={roles.error}>
        <div class="error-state" role="alert" style="margin-bottom: 14px;">
          Prompt-role discovery is unavailable. Role options may be incomplete.
          <button class="ghost small" onClick={() => refetchRoles()}>Retry</button>
          <details><summary>Details</summary><pre class="mono">{String(roles.error)}</pre></details>
        </div>
      </Show>

      {/* ---------------------------------- Filter Bar & Layout Grid -- */}
      <div class="prompts-grid">
        {/* ------------------------------------------ editable settings -- */}
        <div class="panel">
          <div class="panel-title-row">
            <div>
              <h2>Editing {scopeLabel(props.scope())} Blocks</h2>
              <p class="dim">
                Universal agent harness prompt blocks. Inspect original prompts or override per workspace.
              </p>
            </div>
            <button
              class="ghost small"
              onClick={() => setShowHistory(!showHistory())}
            >
              Revision history ({history().length})
            </button>
          </div>

          {/* Clean Segmented Filter Strip */}
          <div class="prompt-blocks-filter">
            <button
              class="prompt-filter-btn"
              classList={{ active: blockFilter() === "all" }}
              onClick={() => setBlockFilter("all")}
            >
              All Blocks ({BLOCKS.length + 1})
            </button>
            <For each={BLOCKS}>
              {(b) => (
                <button
                  class={`prompt-filter-btn block-${b.id}`}
                  classList={{ active: blockFilter() === b.id }}
                  onClick={() => setBlockFilter(b.id)}
                >
                  <span class={`dot ${b.dotClass}`} style="margin-right: 5px;" />
                  {b.label}
                </button>
              )}
            </For>
            <button
              class="prompt-filter-btn block-code-owned"
              classList={{ active: blockFilter() === "code-owned" }}
              onClick={() => setBlockFilter("code-owned")}
            >
              <span class="dot dot-code" style="margin-right: 5px;" />
              Code-owned
            </button>
          </div>

          {/* Optional History Drawer */}
          <Show when={showHistory()}>
            <div class="prompt-history-drawer">
              <div class="prompt-history-head">
                <strong>Recent Prompt Edits (This Session)</strong>
                <button class="ghost small" onClick={() => setShowHistory(false)}>Close</button>
              </div>
              <Show
                when={history().length > 0}
                fallback={<p class="dim small" style="margin: 4px 0;">No edits recorded yet in this session.</p>}
              >
                <div class="prompt-history-list">
                  <For each={history()}>
                    {(item) => (
                      <div class="prompt-history-item">
                        <div class="prompt-history-meta">
                          <span class="mono dim">{item.timestamp}</span>
                          <strong>{item.block}</strong>
                          <span class="chip chip-kind">{item.scope}</span>
                          <span class="dim small mono">{item.chars} chars</span>
                        </div>
                        <button
                          class="ghost small"
                          onClick={() => {
                            setDraft(item.text);
                            setEditing(item.block);
                            props.pushToast("info", `Restored ${item.block} revision into editor`);
                          }}
                        >
                          Restore
                        </button>
                      </div>
                    )}
                  </For>
                </div>
              </Show>
            </div>
          </Show>

          {/* Prompt Blocks */}
          <For each={BLOCKS}>
            {(block) => {
              const own = () => blockText(block.id);
              const baseline = () => inheritedText(block.id);
              const from = () => contributors(block.id);
              const isVisible = () => blockFilter() === "all" || blockFilter() === block.id;

              return (
                <Show when={isVisible()}>
                  <div
                    class={`prompt-block-card block-${block.id}`}
                    classList={{ editing: editing() === block.id }}
                  >
                    <div class="prompt-block-card-head">
                      <h3>
                        <span class={`dot ${block.dotClass}`} />
                        {block.label}
                        <Show
                          when={own() !== null}
                          fallback={
                            <span class="chip chip-kind" title="Inherited from higher tier">
                              Inherited from {from().map((d) => LAYER_LABELS[d.layer]).join(", ") || "shipped default"}
                            </span>
                          }
                        >
                          <span class="chip chip-ok">Custom ({scopeLabel(props.scope())})</span>
                        </Show>
                      </h3>

                      <div class="prompt-block-meta">
                        <Show when={own() !== null}>
                          <span class="dim small mono">{own()!.length} chars</span>
                        </Show>
                        <Show when={own() === null && baseline() !== null}>
                          <span class="dim small mono">{baseline()!.length} chars (baseline)</span>
                        </Show>
                        <Show when={editing() !== block.id}>
                          <button
                            class="ghost small"
                            onClick={(e) => {
                              e.stopPropagation();
                              startOverride(block.id);
                            }}
                          >
                            {own() === null ? "Override" : "Edit"}
                          </button>
                          <Show when={own() !== null}>
                            <button
                              class="ghost small"
                              disabled={busy()}
                              onClick={(e) => {
                                e.stopPropagation();
                                void reset(block.id);
                              }}
                            >
                              Reset
                            </button>
                          </Show>
                        </Show>
                      </div>
                    </div>

                    <p class="dim small" style="margin: 0 0 8px;">{block.help}</p>

                    <Show
                      when={editing() === block.id}
                      fallback={
                        <>
                          {/* When NOT overridden, display the ACTIVE ORIGINAL PROMPT TEXT so users can actually see it! */}
                          <Show when={own() !== null}>
                            <pre class="mono prompt-preview">{own()}</pre>
                          </Show>

                          <Show when={own() === null}>
                            <Show
                              when={baseline() !== null}
                              fallback={
                                <p class="dim small" style="margin-top: 4px;">
                                  Nothing set anywhere. Click Override to define {block.label} for {scopeLabel(props.scope())}.
                                </p>
                              }
                            >
                              <div class="prompt-inherited-preview">
                                <div class="prompt-inherited-banner">
                                  <span>Active baseline ({from().map((d) => LAYER_LABELS[d.layer]).join(", ") || "shipped default"})</span>
                                  <span>{baseline()!.length} characters</span>
                                </div>
                                <pre class="prompt-inherited-text">{baseline()}</pre>
                              </div>
                            </Show>
                          </Show>
                        </>
                      }
                    >
                      {/* IN EDITING MODE: Show Universal Templates & Baseline Comparison */}
                      <Show when={UNIVERSAL_TEMPLATES[block.id]?.length}>
                        <div class="prompt-template-pills">
                          <span class="prompt-template-label">Templates:</span>
                          <For each={UNIVERSAL_TEMPLATES[block.id]}>
                            {(tmpl) => (
                              <button
                                type="button"
                                class="prompt-template-pill"
                                onClick={() => insertSnippet(tmpl.snippet)}
                              >
                                {tmpl.label}
                              </button>
                            )}
                          </For>
                        </div>
                      </Show>

                      {/* Overriding Baseline Comparison */}
                      <Show when={baseline() !== null}>
                        <details class="prompt-override-baseline">
                          <summary>Compare with baseline ({from().map((d) => LAYER_LABELS[d.layer]).join(", ") || "shipped default"})</summary>
                          <pre>{baseline()}</pre>
                        </details>
                      </Show>

                      <textarea
                        class="prompt-editor"
                        rows={block.id === "identity" ? 8 : 11}
                        value={draft()}
                        onInput={(e) => setDraft(e.currentTarget.value)}
                        placeholder="Plain text or instructions for the universal agent"
                      />

                      <Show when={block.id === "guardrails"}>
                        <div class="prompt-security-callout">
                          <strong>Advisory (Invariants 13 & 16):</strong> Guardrails instruct model reasoning, but do not replace OS sandboxing. For mathematical enforcement boundaries, configure <a href="#/security">Permissions & Sandbox</a>.
                        </div>
                      </Show>

                      <div class="row-gap" style="margin-top: 10px;">
                        <button
                          class="primary small"
                          disabled={busy()}
                          onClick={() => save(block.id)}
                        >
                          {busy() ? "Saving…" : `Save to ${scopeLabel(props.scope())}`}
                        </button>
                        <button
                          class="ghost small"
                          onClick={() => setEditing(null)}
                        >
                          Cancel
                        </button>
                        <span class="dim small mono" style="margin-left: auto;">
                          {draft().length} chars · ~{Math.round(draft().length / 4)} tokens
                        </span>
                      </div>
                    </Show>

                    <Show
                      when={
                        (block.id === "guardrails" || block.id === "surface-note") &&
                        from().length > 0
                      }
                    >
                      <ul class="guardrail-sources">
                        <For each={from()}>
                          {(d) => (
                            <li>
                              <Show
                                when={
                                  (d.layer === "project" && props.scope() === "project") ||
                                  (d.layer === "shared" && props.scope() === "user")
                                }
                                fallback={
                                  <span class="chip chip-kind" title="Inherited from higher tier — cannot be removed here">
                                    {LAYER_LABELS[d.layer]} ({d.bytes}B)
                                  </span>
                                }
                              >
                                <span class="chip chip-ok">{LAYER_LABELS[d.layer]} ({d.bytes}B)</span>
                              </Show>
                            </li>
                          )}
                        </For>
                      </ul>
                    </Show>
                  </div>
                </Show>
              );
            }}
          </For>

          <Show when={blockFilter() === "all" || blockFilter() === "code-owned"}>
            <div class="prompt-block-card block-code-owned code-owned">
              <div class="prompt-block-card-head">
                <h3><span class="dot dot-code" /> Code-owned Interface <span class="chip chip-kind">Immutable</span></h3>
              </div>
              <p class="dim small">
                The capability contract, the <code>Surface:</code> line, and the skill and
                MCP inventories describe the callable interface as it actually is. Editing them
                in the prompt could only make the model misinformed about its runtime tools.
                Use <strong>Surface note</strong> to supply context about where replies land.
              </p>
            </div>
          </Show>
        </div>

        {/* ------------------------------------------------- effective ---- */}
        <div class="panel">
          <div class="panel-title-row">
            <div>
              <h2>Effective Prompt Workbench</h2>
              <p class="dim">Assembled system prompt across active layers.</p>
            </div>
            <button
              class="prompt-copy-btn"
              onClick={() => copyToClipboard(renderedText(), "prompt")}
            >
              <Show when={copiedPrompt()} fallback={<>Copy prompt</>}>
                Copied
              </Show>
            </button>
          </div>

          {/* Surface & Role Switchers */}
          <div class="panel-title-row" style="margin-top: 10px;">
            <div>
              <h3 style="margin: 0; font-size: 13px;">Preview Target</h3>
            </div>
            <Show when={(roles()?.roles ?? []).length > 0}>
              <div style="display: flex; align-items: center; gap: 6px;">
                <span class="dim small">Role:</span>
                <select
                  class="small"
                  value={previewRole()}
                  onChange={(e) => setPreviewRole(e.currentTarget.value)}
                >
                  <option value="">(default — universal)</option>
                  <For each={roles()!.roles}>
                    {(r) => <option value={r}>{r}</option>}
                  </For>
                </select>
              </div>
            </Show>
          </div>

          <div class="prompt-surface-pills" style="margin-top: 8px;">
            <For each={SURFACES}>
              {(s) => (
                <button
                  class="prompt-surface-btn"
                  classList={{ active: previewSurface() === s.id }}
                  onClick={() => {
                    setPreviewSurface(s.id);
                    scrollToSurface();
                  }}
                >
                  {s.label}
                </button>
              )}
            </For>
          </div>

          {/* Prominent Live Target Injections Callout — Instantly updates on click! */}
          <div class="preview-target-diff">
            <div class="preview-target-diff-header">
              <div style="display: flex; align-items: center; gap: 6px; flex-wrap: wrap;">
                <span class="chip chip-tone-info">Active Surface: {SURFACES.find((s) => s.id === previewSurface())?.label ?? previewSurface()}</span>
                <Show when={previewRole()}>
                  <span class="chip chip-tone-accent">Role: {previewRole()}</span>
                </Show>
                <span class="dim small mono">{tokens()} tokens</span>
              </div>
              <button
                class="ghost small"
                onClick={scrollToSurface}
                title="Scroll down to the injected Surface block in the prompt preview"
              >
                Jump to Surface line
              </button>
            </div>
            <div class="preview-target-diff-body">
              <div class="preview-diff-line">
                <span class="preview-diff-tag">Injected Surface Line:</span>
                <code>{surfaceLine()}</code>
              </div>
              <Show when={roleInstruction()}>
                <div class="preview-diff-line">
                  <span class="preview-diff-tag">Injected Role Directives:</span>
                  <code>{roleInstruction()}</code>
                </div>
              </Show>
            </div>
          </div>

          {/* Quick Target Section Jump Strip */}
          <div class="prompt-jump-strip" style="margin-top: 12px;">
            <span class="dim small" style="margin-right: 2px;">Section:</span>
            <For each={BLOCKS}>
              {(b) => (
                <button
                  class="prompt-jump-btn"
                  classList={{ active: selectedTableBlock() === b.id }}
                  onClick={() => {
                    setSelectedTableBlock(b.id);
                    setPreviewViewMode("sections");
                  }}
                >
                  <span class={`dot ${b.dotClass}`} style="margin-right: 4px;" />
                  {b.label}
                </button>
              )}
            </For>
            <button
              class="prompt-jump-btn"
              classList={{ active: selectedTableBlock() === "surface" }}
              onClick={() => {
                setSelectedTableBlock("surface");
                setPreviewViewMode("sections");
              }}
            >
              <span class="dot dot-surface" style="margin-right: 4px;" />
              Surface Target
            </button>
            <button
              class="prompt-jump-btn"
              classList={{ active: previewViewMode() === "full" && selectedTableBlock() === null }}
              onClick={() => {
                setSelectedTableBlock(null);
                setPreviewViewMode("full");
                setSearchQuery("");
                if (promptPreRef) {
                  promptPreRef.scrollTop = 0;
                }
              }}
            >
              Full prompt
            </button>
          </div>

          <Show when={overBudget()}>
            <p class="posture-warning small" style="margin-top: 6px;">
              This prompt is injected on every turn of every session. Over ~1,500 tokens it
              becomes a recurring context cost worth trimming.
            </p>
          </Show>

          {/* Stacked Proportional Provenance Bar */}
          <div class="prompt-provenance-stacked-bar" title="Prompt layer size distribution">
            <For each={(preview() ?? effective())?.layers ?? []}>
              {(d) => {
                const pct = Math.max(2, Math.round((d.bytes / totalBytes()) * 100));
                return (
                  <div
                    class={`prompt-provenance-segment ${LAYER_CSS_CLASS[d.layer] || "seed"}`}
                    style={{ width: `${pct}%` }}
                    title={`${d.block} (${LAYER_LABELS[d.layer]}): ${d.bytes}B (${pct}%)`}
                    onClick={() => {
                      setSelectedTableBlock(d.block);
                      setPreviewViewMode("sections");
                    }}
                  />
                );
              }}
            </For>
          </div>

          {/* Provenance Table */}
          <table class="table" style="margin-bottom: 14px;">
            <thead>
              <tr>
                <th>Block</th>
                <th>Layer Source</th>
                <th>Bytes</th>
                <th>Share</th>
              </tr>
            </thead>
            <tbody>
              <For each={(preview() ?? effective())?.layers ?? []}>
                {(d) => {
                  const pct = Math.round((d.bytes / totalBytes()) * 100);
                  const isSelected = () => selectedTableBlock() === d.block;
                  return (
                    <tr
                      class={isSelected() ? "active-row" : ""}
                      style="cursor: pointer;"
                      onClick={() => {
                        setSelectedTableBlock(d.block);
                        setPreviewViewMode("sections");
                      }}
                      title="Click to view this block in section view"
                    >
                      <td>
                        <strong>{d.block}</strong>
                        <Show when={isSelected()}>
                          <span class="chip chip-ok" style="margin-left: 6px; font-size: 12px;">viewing</span>
                        </Show>
                      </td>
                      <td>
                        <span class={`chip chip-tone-${d.layer === "seed" ? "neutral" : d.layer === "shared" ? "accent" : "info"}`} title={d.source ?? "built-in"}>
                          {LAYER_LABELS[d.layer]}
                        </span>
                      </td>
                      <td class="mono">{d.bytes}B</td>
                      <td class="dim mono">{pct}%</td>
                    </tr>
                  );
                }}
              </For>
            </tbody>
          </table>

          {/* View Mode Switcher */}
          <div style="display: flex; align-items: center; justify-content: space-between; margin-top: 10px; margin-bottom: 8px;">
            <div class="preview-view-mode-tabs">
              <button
                class="preview-mode-btn"
                classList={{ active: previewViewMode() === "full" }}
                onClick={() => setPreviewViewMode("full")}
              >
                Full Assembled
              </button>
              <button
                class="preview-mode-btn"
                classList={{ active: previewViewMode() === "sections" }}
                onClick={() => setPreviewViewMode("sections")}
              >
                By Sections
              </button>
            </div>
            <Show when={previewViewMode() === "full"}>
              <div class="prompt-search-wrapper" style="max-width: 220px;">
                <input
                  type="text"
                  class="prompt-search-input"
                  placeholder="Search in prompt…"
                  value={searchQuery()}
                  onInput={(e) => setSearchQuery(e.currentTarget.value)}
                />
                <Show when={searchQuery()}>
                  <button
                    class="prompt-search-clear"
                    onClick={() => setSearchQuery("")}
                  >
                    Clear
                  </button>
                </Show>
              </div>
            </Show>
          </div>

          {/* Section Breakdown View Mode */}
          <Show when={previewViewMode() === "sections"}>
            <div style="max-height: 480px; overflow-y: auto;">
              <For each={BLOCKS}>
                {(b) => {
                  const content = () => (preview() ?? effective())?.blocks?.[b.id] ?? (preview() ?? effective())?.seed_blocks?.[b.id] ?? "";
                  const isHighlighted = () => selectedTableBlock() === b.id;
                  return (
                    <div
                      class={`preview-section-card block-${b.id}`}
                      style={isHighlighted() ? { "border-color": "var(--accent)", "box-shadow": "0 0 0 1px var(--accent)" } : {}}
                    >
                      <div class="preview-section-card-head">
                        <span><span class={`dot ${b.dotClass}`} style="margin-right: 5px;" />{b.label}</span>
                        <span class="dim small mono">{content().length} chars · ~{Math.round(content().length / 4)} tokens</span>
                      </div>
                      <pre>{content() || "(Not defined in this layer)"}</pre>
                    </div>
                  );
                }}
              </For>

              {/* Surface Section */}
              <div
                class="preview-section-card block-surface-note"
                style={selectedTableBlock() === "surface" || selectedTableBlock() === "surface-note" ? { "border-color": "var(--accent)", "box-shadow": "0 0 0 1px var(--accent)" } : {}}
              >
                <div class="preview-section-card-head">
                  <span><span class="dot dot-surface" style="margin-right: 5px;" />Surface & Target Context</span>
                  <span class="chip chip-tone-info">{SURFACES.find((s) => s.id === previewSurface())?.label ?? previewSurface()}</span>
                </div>
                <pre>{surfaceLine()}{roleInstruction() ? "\n\n" + roleInstruction() : ""}</pre>
              </div>
            </div>
          </Show>

          {/* Full Rendered Prompt Text */}
          <Show when={previewViewMode() === "full"}>
            <Show
              when={!searchQuery()}
              fallback={
                <div class="prompt-full mono" style="max-height: 460px; overflow-y: auto;">
                  <Show
                    when={searchMatches().length > 0}
                    fallback={<p class="dim" style="padding: 12px;">No lines match "{searchQuery()}".</p>}
                  >
                    <For each={searchMatches()}>
                      {(item) => (
                        <div style="padding: 2px 0; border-bottom: 1px solid var(--border-soft);">
                          <span class="dim" style="display: inline-block; width: 36px;">{item.num}:</span>
                          <span>{item.line}</span>
                        </div>
                      )}
                    </For>
                  </Show>
                </div>
              }
            >
              <pre
                ref={promptPreRef}
                class="mono prompt-full"
                style="max-height: 460px; overflow-y: auto; scroll-behavior: smooth;"
              >
                {renderedText()}
              </pre>
            </Show>
          </Show>

          <Show when={effective()}>
            <p class="dim small" style="margin-top: 10px;">
              Fingerprint <code>{currentFingerprint().slice(0, 16)}</code> · Changes apply from the next turn of every session; a turn already running keeps the prompt it started with.
            </p>
          </Show>
        </div>
      </div>
    </section>
  );
}
