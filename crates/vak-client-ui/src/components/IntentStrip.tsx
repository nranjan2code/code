/// What vak read your request as, shown before you send it
/// (docs/design/47-commitment-kernel.md).
///
/// Implemented as a high-density, theme-aware Pre-Flight HUD adhering strictly
/// to Vak design tokens and typography. Preserves 100% of telemetry (axes,
/// operational narrowing limits, float signal weights, and session history)
/// with granular expand/collapse and dismissal controls.

import { For, Show, createEffect, createSignal, onCleanup } from "solid-js";
import { explainIntent, type IntentExplain } from "../api";
import Icon from "./Icon";

const DEBOUNCE_MS = 260;
const MIN_CHARS = 12;

type Tone = "quiet" | "notice" | "warn";

function toneOf(intent: IntentExplain): Tone {
  const { reading, engagement } = intent;
  if (
    reading.stakes === "irreversible" ||
    engagement.posture.hil === "defer" ||
    engagement.posture.clarify === "ask"
  ) {
    return "warn";
  }
  if (
    engagement.posture.open_commitment ||
    reading.evidence === "verified" ||
    reading.evidence === "audited" ||
    engagement.limits.approval_ceiling === "ask"
  ) {
    return "notice";
  }
  return "quiet";
}

function summary(intent: IntentExplain): string {
  const { reading, engagement } = intent;
  const parts: string[] = [reading.act];
  if (engagement.posture.open_commitment) parts.push("opens a commitment");
  else if (engagement.posture.managed) parts.push("managed");
  const caps = engagement.limits.capabilities;
  if (caps.kind === "only") {
    parts.push(caps.names.length === 0 ? "no tools" : `${caps.names.length} tools`);
  }
  if (engagement.limits.approval_ceiling === "ask") parts.push("approval required");
  if (engagement.posture.hil === "defer") parts.push("questions queued");
  if (reading.evidence !== "none") parts.push(`${reading.evidence} evidence`);
  return parts.join(" · ");
}

export function IntentStrip(props: {
  prompt: string;
  sessionId?: string | null;
  disabled?: boolean;
}) {
  const [intent, setIntent] = createSignal<IntentExplain | null>(null);
  const [intentError, setIntentError] = createSignal("");
  const [open, setOpen] = createSignal(
    localStorage.getItem("vak.intent.hud_expanded") === "true",
  );
  const [dismissed, setDismissed] = createSignal(false);

  function toggleOpen() {
    const next = !open();
    setOpen(next);
    try {
      localStorage.setItem("vak.intent.hud_expanded", String(next));
    } catch {
      // Storage unavailable or disabled
    }
  }

  createEffect(() => {
    const prompt = props.prompt.trim();
    if (props.disabled || prompt.length < MIN_CHARS || dismissed()) {
      setIntent(null);
      setIntentError("");
      return;
    }
    setIntentError("");
    const controller = new AbortController();
    const timer = setTimeout(() => {
      void explainIntent(prompt, props.sessionId, controller.signal)
        .then(setIntent)
        .catch((error) => {
          if (error instanceof DOMException && error.name === "AbortError") return;
          setIntentError(`Intent analysis unavailable: ${error instanceof Error ? error.message : String(error)}`);
        });
    }, DEBOUNCE_MS);
    onCleanup(() => {
      clearTimeout(timer);
      controller.abort();
    });
  });

  // Re-enable if prompt resets or changes substantially
  createEffect(() => {
    if (props.prompt.trim().length === 0 && dismissed()) {
      setDismissed(false);
    }
  });

  return (
    <>
      <Show when={intentError()}>
        <div class="intent-hud-error" role="status">{intentError()} You can still send the prompt.</div>
      </Show>
      <Show when={intent()}>
      {(current) => (
        <div
          class={`intent-hud intent-hud-${toneOf(current())}`}
          classList={{ "intent-hud-open": open() }}
        >
          {/* Header Strip & Compact Ticker */}
          <div class="intent-hud-header">
            <button
              class="intent-hud-toggle"
              aria-expanded={open()}
              onClick={toggleOpen}
              title={open() ? "Collapse intent HUD" : "Expand full intent telemetry"}
            >
              <span class="intent-hud-mark" aria-hidden="true" />
              <span class="intent-hud-summary">{summary(current())}</span>
              <Show when={!current().provenance.reproducible}>
                <span
                  class="intent-hud-inferred"
                  title="Classified via model tier; not fully reproducible from ledger"
                >
                  inferred
                </span>
              </Show>
              <span
                class="intent-hud-chev"
                classList={{ "intent-hud-chev-open": open() }}
              >
                <Icon name="chevron" size={12} />
              </span>
            </button>

            <div class="intent-hud-controls">
              <button
                class="intent-hud-btn"
                onClick={() => setDismissed(true)}
                title="Hide intent card for this prompt"
                aria-label="Dismiss intent card"
              >
                <span class="intent-hud-btn-label">hide</span>
              </button>
            </div>
          </div>

          {/* High-stakes warning banner (visible even if collapsed) */}
          <Show when={toneOf(current()) === "warn" && current().engagement.posture.note}>
            {(note) => <div class="intent-hud-note">{note().split("\n")[0]}</div>}
          </Show>

          {/* Full Expanded Lossless Telemetry Display */}
          <Show when={open()}>
            <div class="intent-hud-body">
              {/* Context Ribbon */}
              <Show when={current().history}>
                {(hist) => (
                  <div class="intent-hud-context">
                    <span class="intent-hud-context-lead">Context</span>
                    <span class="intent-hud-context-val">
                      turn {hist().turn_index + 1}
                      <Show when={hist().previous_act}>
                        {(prev) => ` · continues ${prev()}`}
                      </Show>
                      <Show when={hist().commitment_open}> · commitment active</Show>
                    </span>
                  </div>
                )}
              </Show>

              {/* 6-Axis Telemetry Grid */}
              <div class="intent-hud-axes">
                <For
                  each={[
                    ["act", current().reading.act],
                    ["horizon", current().reading.horizon],
                    ["stakes", current().reading.stakes],
                    ["evidence", current().reading.evidence],
                    ["clarity", current().reading.clarity],
                    ["attendance", current().reading.attendance],
                  ] as [string, string][]}
                >
                  {([k, v]) => (
                    <div class="intent-hud-axis-cell">
                      <span class="intent-hud-axis-key">{k}</span>
                      <span class={`intent-hud-axis-val axis-${v}`}>{v}</span>
                    </div>
                  )}
                </For>
              </div>

              {/* Operational Narrowing Limits */}
              <div class="intent-hud-limits">
                <span class="intent-hud-limits-lead">Narrowed Limits</span>
                <Show
                  when={current().narrows.length > 0}
                  fallback={
                    <span class="intent-hud-limits-empty">
                      Unrestricted · baseline rules apply
                    </span>
                  }
                >
                  <div class="intent-hud-limits-list">
                    <For each={current().narrows}>
                      {(line) => <span class="intent-hud-limit-tag">{line}</span>}
                    </For>
                  </div>
                </Show>
              </div>

              {/* Signals Spectrum Tray */}
              <div class="intent-hud-signals">
                <span class="intent-hud-signals-lead">
                  Signals ({current().provenance.signals.length}) · resolved by{" "}
                  {current().provenance.tier}
                </span>
                <div class="intent-hud-signals-list">
                  <For each={current().provenance.signals}>
                    {(sig) => (
                      <div class="intent-hud-signal-item">
                        <span class="sig-kind">{sig.kind}</span>
                        <span class="sig-weight">{sig.weight.toFixed(2)}</span>
                        <span class="sig-detail">{sig.detail}</span>
                      </div>
                    )}
                  </For>
                </div>
              </div>
            </div>
          </Show>
        </div>
      )}
      </Show>
    </>
  );
}
