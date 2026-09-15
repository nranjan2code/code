import { For, Show, createEffect, createMemo, createResource, createSignal } from "solid-js";

import { api } from "./api";
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

const BLOCKS: { id: PromptBlock; label: string; help: string }[] = [
  {
    id: "identity",
    label: "Identity",
    help: "Who the agent is. The narrowest setting that defines this wins.",
  },
  {
    id: "operating-rules",
    label: "Operating rules",
    help: "How it works. The narrowest setting that defines this wins.",
  },
  {
    id: "guardrails",
    label: "Guardrails",
    help: "Guardrails from every scope apply together. A narrower scope cannot remove one.",
  },
  {
    id: "surface-note",
    label: "Surface note",
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

const SURFACES = ["cli", "desktop", "server", "background", "subagent", "telegram"] as const;

const TEMPLATES: Record<PromptBlock, { label: string; snippet: string }[]> = {
  "guardrails": [
    { label: "+ Confirm file deletion", snippet: "- Always ask for human confirmation before deleting files or directories" },
    { label: "+ Never leak secrets", snippet: "- Never print, log, or export credentials, API keys, or private tokens" },
    { label: "+ Strict TypeScript", snippet: "- Strict TypeScript mode: ensure code compiles with zero type errors" },
    { label: "+ Concise JSON only", snippet: "- Format output strictly as valid compact JSON without markdown formatting" },
  ],
  "surface-note": [
    { label: "+ Public channel warning", snippet: "- This is a shared channel; assume anyone can inspect the conversation" },
    { label: "+ Autonomous background task", snippet: "- Running unattended: prioritize non-blocking, idempotent operations" },
    { label: "+ Read-only observation", snippet: "- Perform inspection only: do not perform any destructive actions" },
  ],
  "operating-rules": [
    { label: "+ Verify before editing", snippet: "- Always inspect file content before applying targeted edits" },
    { label: "+ Outcome-directed prose", snippet: "- Keep explanations concise, technical, and focused on outcomes" },
  ],
  "identity": [
    { label: "+ Senior Systems Engineer", snippet: "You are an expert systems software engineer and systems architect specializing in Rust and distributed systems." },
    { label: "+ Production Site SRE", snippet: "You are a production site reliability engineer focused on operational resilience and diagnostics." },
  ],
};

function scopeLabel(scope: ConfigScope): string {
  return scope === "user" ? "Global" : "Workspace";
}

export function PromptsSection(props: {
  scope: () => ConfigScope;
  pushToast: (kind: "info" | "warn" | "alert", text: string) => void;
  onScopeChange?: (scope: ConfigScope) => void;
}) {
  const [layer, { refetch: refetchLayer }] = createResource(
    () => props.scope(),
    (scope) => api.promptLayer(scope),
  );
  const [effective, { refetch: refetchEffective }] = createResource<PromptEffective>(
    () => api.promptEffective(),
  );
  const [roles, { refetch: refetchRoles }] = createResource(() => api.promptRoles());

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

  const blockText = (block: PromptBlock): string | null => {
    const l = layer()?.layer;
    if (!l) return null;
    if (block === "identity") return l.identity ?? null;
    if (block === "operating-rules") return l.operating_rules ?? null;
    const rules = (block === "guardrails" ? l.guardrails : l.surface_notes) ?? [];
    return rules.length ? rules.map((r) => `- ${r}`).join("\n") : null;
  };

  const contributors = (block: PromptBlock): PromptLayerDescriptor[] =>
    (effective()?.layers ?? []).filter((d) => d.block === block);

  async function save(block: PromptBlock) {
    setBusy(true);
    try {
      await api.putPromptBlock(props.scope(), block, draft());
      props.pushToast(
        "info",
        `${block} saved to ${scopeLabel(props.scope())}. Applies to new sessions.`,
      );
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
      await api.putPromptBlock(props.scope(), block, null);
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
      const res = await api.promptPreview(surface, role || undefined);
      setPreview(res);
    } catch (e) {
      props.pushToast("alert", `Preview failed: ${(e as Error).message}`);
    }
  }

  // Trigger preview update when surface or role changes
  createEffect(() => {
    const s = previewSurface();
    const r = previewRole();
    void runPreview(s, r);
  });

  const tokens = () => preview()?.estimated_tokens ?? effective()?.estimated_tokens ?? 0;
  const overBudget = () => tokens() > 1500;
  const currentFingerprint = () => preview()?.fingerprint ?? effective()?.fingerprint ?? "";

  const renderedText = () => preview()?.text ?? effective()?.text ?? "";

  // Filtered preview lines or match count
  const searchMatches = createMemo(() => {
    const q = searchQuery().trim().toLowerCase();
    if (!q) return [];
    const lines = renderedText().split("\n");
    return lines
      .map((line, idx) => ({ line, num: idx + 1 }))
      .filter((item) => item.line.toLowerCase().includes(q));
  });

  const totalBytes = createMemo(() => {
    const layers = effective()?.layers ?? [];
    return layers.reduce((sum, d) => sum + d.bytes, 0) || 1;
  });

  const activeLayersCount = createMemo(() => {
    const layers = effective()?.layers ?? [];
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
          <div class="prompt-kpi-val">{effective()?.layers.length ?? 0} slices</div>
          <div class="prompt-kpi-detail">
            {effective()?.layers.map((l) => LAYER_LABELS[l.layer]).slice(0, 3).join(" · ") || "Seed only"}
          </div>
        </div>

        <div class="prompt-kpi-card">
          <div class="prompt-kpi-head">
            <span>Fingerprint</span>
            <button
              class="ghost small"
              style="padding: 1px 6px; font-size: 11px;"
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
                Only what {scopeLabel(props.scope())} defines. Narrowest scope wins for identity and rules; guardrails and notes accumulate.
              </p>
            </div>
          </div>

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
                  class="prompt-filter-btn"
                  classList={{ active: blockFilter() === b.id }}
                  onClick={() => setBlockFilter(b.id)}
                >
                  {b.label}
                </button>
              )}
            </For>
            <button
              class="prompt-filter-btn"
              classList={{ active: blockFilter() === "code-owned" }}
              onClick={() => setBlockFilter("code-owned")}
            >
              Code-owned 🔒
            </button>
          </div>

          <For each={BLOCKS}>
            {(block) => {
              const own = () => blockText(block.id);
              const from = () => contributors(block.id);
              const isVisible = () => blockFilter() === "all" || blockFilter() === block.id;

              return (
                <Show when={isVisible()}>
                  <div
                    class="prompt-block-card"
                    classList={{ editing: editing() === block.id }}
                  >
                    <div class="prompt-block-card-head">
                      <h3>
                        {block.label}
                        <Show
                          when={own() !== null}
                          fallback={<span class="chip chip-kind">inherited</span>}
                        >
                          <span class="chip chip-ok">set in {scopeLabel(props.scope())}</span>
                        </Show>
                      </h3>

                      <div class="prompt-block-meta">
                        <Show when={own() !== null}>
                          <span class="dim small mono">{own()!.length} chars</span>
                        </Show>
                        <Show when={editing() !== block.id}>
                          <button
                            class="ghost small"
                            onClick={() => {
                              setDraft(own() ?? "");
                              setEditing(block.id);
                            }}
                          >
                            {own() === null ? "Override" : "Edit"}
                          </button>
                          <Show when={own() !== null}>
                            <button
                              class="ghost small"
                              disabled={busy()}
                              onClick={() => reset(block.id)}
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
                          <Show
                            when={own() !== null}
                            fallback={
                              <p class="dim small" style="margin-top: 4px;">
                                <Show
                                  when={from().length}
                                  fallback={<>Nothing set anywhere.</>}
                                >
                                  Inheriting from{" "}
                                  <strong>
                                    {from().map((d) => LAYER_LABELS[d.layer]).join(", ")}
                                  </strong>
                                  . Click Override to define workspace-level rules.
                                </Show>
                              </p>
                            }
                          >
                            <pre class="mono prompt-preview">{own()}</pre>
                          </Show>
                        </>
                      }
                    >
                      {/* Quick Template Pills */}
                      <Show when={TEMPLATES[block.id]?.length}>
                        <div class="prompt-template-pills">
                          <span class="prompt-template-label">Snippets:</span>
                          <For each={TEMPLATES[block.id]}>
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

                      <textarea
                        class="prompt-editor"
                        rows={block.id === "identity" ? 7 : 10}
                        value={draft()}
                        onInput={(e) => setDraft(e.currentTarget.value)}
                        placeholder={
                          block.id === "guardrails"
                            ? "- one guardrail per line\n- e.g. - Always ask for confirmation before modifying production resources"
                            : block.id === "surface-note"
                              ? "- this is a public channel; assume anyone can read the reply"
                              : "Plain text or markdown instructions for the model"
                        }
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
                                    🔒 {LAYER_LABELS[d.layer]} ({d.bytes}B)
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
            <div class="prompt-block-card code-owned">
              <div class="prompt-block-card-head">
                <h3>Code-owned Interface <span class="chip chip-kind">🔒 Immutable</span></h3>
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
              <p class="dim">What the model actually receives across active layers.</p>
            </div>
            <button
              class="prompt-copy-btn"
              onClick={() => copyToClipboard(renderedText(), "prompt")}
            >
              <Show when={copiedPrompt()} fallback={<>📋 Copy Full Prompt</>}>
                ✓ Copied!
              </Show>
            </button>
          </div>

          <Show when={overBudget()}>
            <p class="posture-warning small" style="margin-top: 6px;">
              ⚠️ This prompt is injected on every turn of every session. Over ~1,500 tokens it
              becomes a recurring context cost worth trimming.
            </p>
          </Show>

          {/* Stacked Proportional Provenance Bar */}
          <div class="prompt-provenance-stacked-bar" title="Prompt layer size distribution">
            <For each={effective()?.layers ?? []}>
              {(d) => {
                const pct = Math.max(2, Math.round((d.bytes / totalBytes()) * 100));
                return (
                  <div
                    class={`prompt-provenance-segment ${LAYER_CSS_CLASS[d.layer] || "seed"}`}
                    style={{ width: `${pct}%` }}
                    title={`${d.block} (${LAYER_LABELS[d.layer]}): ${d.bytes}B (${pct}%)`}
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
              <For each={effective()?.layers ?? []}>
                {(d) => {
                  const pct = Math.round((d.bytes / totalBytes()) * 100);
                  return (
                    <tr>
                      <td><strong>{d.block}</strong></td>
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

          {/* Surface & Role Switchers */}
          <div class="panel-title-row" style="margin-top: 16px;">
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
                  <option value="">(default — no role)</option>
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
                  classList={{ active: previewSurface() === s }}
                  onClick={() => setPreviewSurface(s)}
                >
                  {s}
                </button>
              )}
            </For>
          </div>

          {/* In-Prompt Search Toolbar */}
          <div class="prompt-preview-toolbar">
            <div class="prompt-search-wrapper">
              <input
                type="text"
                class="prompt-search-input"
                placeholder="Filter or search in prompt…"
                value={searchQuery()}
                onInput={(e) => setSearchQuery(e.currentTarget.value)}
              />
              <Show when={searchQuery()}>
                <button
                  class="prompt-search-clear"
                  onClick={() => setSearchQuery("")}
                >
                  ✕
                </button>
              </Show>
            </div>
            <Show when={searchQuery()}>
              <span class="chip chip-kind">
                {searchMatches().length} lines match
              </span>
            </Show>
          </div>

          {/* Rendered Prompt Text */}
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
            <pre class="mono prompt-full" style="max-height: 460px; overflow-y: auto;">
              {renderedText()}
            </pre>
          </Show>

          <Show when={effective()}>
            <p class="dim small" style="margin-top: 10px;">
              Fingerprint <code>{effective()!.fingerprint.slice(0, 16)}</code> · Changes apply to new sessions; active turns keep their admission prompt.
            </p>
          </Show>
        </div>
      </div>
    </section>
  );
}
