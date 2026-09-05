import { For, Show, createResource, createSignal } from "solid-js";

import { api } from "./api";
import type {
  ConfigScope,
  PromptBlock,
  PromptEffective,
  PromptLayerDescriptor,
} from "./types";

/// Prompt layers (docs/design/45-prompt-layers.md).
///
/// Two panes, always. Composition runs through seven tiers, so an editor that
/// showed only your own layer would teach the wrong model of the system —
/// doc 44 requires every editable surface to report the selected layer *and*
/// the effective result with provenance.

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
  seed: "shipped default",
  shared: "Global",
  project: "Workspace",
  surface: "surface",
  bot: "bot",
  chat: "chat",
  agent: "agent role",
};

const SURFACES = ["cli", "desktop", "server", "background", "subagent", "telegram"];

function scopeLabel(scope: ConfigScope): string {
  return scope === "user" ? "Global" : "Workspace";
}

export function PromptsSection(props: {
  scope: () => ConfigScope;
  pushToast: (kind: "info" | "warn" | "alert", text: string) => void;
}) {
  const [layer, { refetch: refetchLayer }] = createResource(
    () => props.scope(),
    (scope) => api.promptLayer(scope),
  );
  const [effective, { refetch: refetchEffective }] = createResource<PromptEffective>(
    () => api.promptEffective(),
  );
  const [roles] = createResource(() => api.promptRoles().catch(() => ({ roles: [] })));

  const [editing, setEditing] = createSignal<PromptBlock | null>(null);
  const [draft, setDraft] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [previewSurface, setPreviewSurface] = createSignal("cli");
  const [previewRole, setPreviewRole] = createSignal("");
  const [preview, setPreview] = createSignal<PromptEffective | null>(null);

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

  async function runPreview() {
    try {
      setPreview(await api.promptPreview(previewSurface(), previewRole() || undefined));
    } catch (e) {
      props.pushToast("alert", `Preview failed: ${(e as Error).message}`);
    }
  }

  const tokens = () => preview()?.estimated_tokens ?? effective()?.estimated_tokens ?? 0;
  const overBudget = () => tokens() > 1500;

  return (
    <section class="prompts-section">
      <p class="dim">
        Editing <strong>{scopeLabel(props.scope())}</strong>
        <Show when={layer()?.path}> · <code>{layer()!.path}</code></Show>
      </p>

      <div class="prompts-grid">
        {/* ------------------------------------------ editable settings -- */}
        <div class="panel">
          <div class="panel-title-row">
            <div>
              <h2>Editing {scopeLabel(props.scope())} settings</h2>
              <p>Only what {scopeLabel(props.scope())} sets. Everything else is inherited.</p>
            </div>
          </div>

          <For each={BLOCKS}>
            {(block) => {
              const own = () => blockText(block.id);
              const from = () => contributors(block.id);
              return (
                <div class="prompt-block">
                  <div class="prompt-block-head">
                    <h3>{block.label}</h3>
                    <Show
                      when={own() !== null}
                      fallback={<span class="chip chip-kind">inherited</span>}
                    >
                      <span class="chip chip-ok">set here</span>
                    </Show>
                  </div>
                  <p class="dim small">{block.help}</p>

                  <Show
                    when={editing() === block.id}
                    fallback={
                      <>
                        <Show
                          when={own() !== null}
                          fallback={
                            <p class="dim small">
                              <Show
                                when={from().length}
                                fallback={<>Nothing set anywhere.</>}
                              >
                                Coming from{" "}
                                {from().map((d) => LAYER_LABELS[d.layer]).join(", ")}.
                              </Show>
                            </p>
                          }
                        >
                          <pre class="mono prompt-preview">{own()}</pre>
                        </Show>
                        <div class="row-gap">
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
                              Reset to inherited
                            </button>
                          </Show>
                        </div>
                      </>
                    }
                  >
                    <textarea
                      class="prompt-editor"
                      rows={block.id === "identity" ? 8 : 12}
                      value={draft()}
                      onInput={(e) => setDraft(e.currentTarget.value)}
                      placeholder={
                        block.id === "guardrails"
                          ? "- one guardrail per line"
                          : block.id === "surface-note"
                            ? "- this is a public channel; assume anyone can read the reply"
                            : "Plain text or markdown"
                      }
                    />
                    <Show when={block.id === "guardrails"}>
                      {/* The failure this prevents: someone relaxing a real
                          permission rule because they believe the prompt
                          already stops the model. */}
                      <p class="posture-warning small">
                        Guardrails instruct the model. They do not enforce anything —
                        a model can misread or be argued out of one.{" "}
                        <a href="#/security">Permissions and the sandbox</a> are the
                        enforcement boundary.
                      </p>
                    </Show>
                    <div class="row-gap">
                      <button class="primary small" disabled={busy()} onClick={() => save(block.id)}>
                        Save to {scopeLabel(props.scope())}
                      </button>
                      <button class="ghost small" onClick={() => setEditing(null)}>
                        Cancel
                      </button>
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
                            {/* A lock, not a greyed-out delete: the rule is
                                that you cannot remove an inherited guardrail,
                                and the UI should say so rather than dangle a
                                dead control. */}
                            <Show
                              when={
                                (d.layer === "project" && props.scope() === "project") ||
                                (d.layer === "shared" && props.scope() === "user")
                              }
                              fallback={<span class="chip chip-kind" title="Inherited — cannot be removed here">🔒 {LAYER_LABELS[d.layer]}</span>}
                            >
                              <span class="chip chip-ok">{LAYER_LABELS[d.layer]}</span>
                            </Show>
                          </li>
                        )}
                      </For>
                    </ul>
                  </Show>
                </div>
              );
            }}
          </For>

          <div class="prompt-block">
            <div class="prompt-block-head">
              <h3>Code-owned</h3>
              <span class="chip chip-kind">🔒 not editable</span>
            </div>
            <p class="dim small">
              The capability contract, the <code>Surface:</code> line, and the skill and
              MCP lists describe the callable interface as it actually is. Editing them
              could only make the model wrong about its own tools. Use{" "}
              <strong>Surface note</strong> to add to the <code>Surface:</code> line
              without rewriting what the runtime observed.
            </p>
          </div>
        </div>

        {/* ------------------------------------------------- effective ---- */}
        <div class="panel">
          <div class="panel-title-row">
            <div>
              <h2>Effective prompt</h2>
              <p>What the model actually receives, and where each part came from.</p>
            </div>
            <span classList={{ chip: true, "chip-kind": !overBudget(), "chip-warn": overBudget() }}>
              ~{tokens()} tokens
            </span>
          </div>

          <Show when={overBudget()}>
            <p class="posture-warning small">
              This prompt is spent on every turn of every session. Over ~1500 tokens it
              is a standing context cost worth trimming.
            </p>
          </Show>

          <table class="table">
            <thead>
              <tr><th>Block</th><th>From</th><th>Size</th></tr>
            </thead>
            <tbody>
              <For each={effective()?.layers ?? []}>
                {(d) => (
                  <tr>
                    <td>{d.block}</td>
                    <td title={d.source ?? "built in"}>{LAYER_LABELS[d.layer]}</td>
                    <td class="dim">{d.bytes}B</td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>

          <div class="panel-title-row">
            <div><h3>Preview as</h3></div>
          </div>
          <div class="row-gap">
            <select value={previewSurface()} onChange={(e) => setPreviewSurface(e.currentTarget.value)}>
              <For each={SURFACES}>{(s) => <option value={s}>{s}</option>}</For>
            </select>
            <Show when={(roles()?.roles ?? []).length > 0}>
              <select value={previewRole()} onChange={(e) => setPreviewRole(e.currentTarget.value)}>
                <option value="">no role</option>
                <For each={roles()!.roles}>{(r) => <option value={r}>{r}</option>}</For>
              </select>
            </Show>
            <button class="ghost small" onClick={runPreview}>Render</button>
          </div>

          <pre class="mono prompt-full">{preview()?.text ?? effective()?.text ?? ""}</pre>

          <Show when={effective()}>
            <p class="dim small">
              fingerprint <code>{effective()!.fingerprint.slice(0, 16)}</code> — changes
              here apply to new sessions; a running turn keeps the prompt it started
              with.
            </p>
          </Show>
        </div>
      </div>
    </section>
  );
}
