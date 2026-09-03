/// What vak read your request as, shown before you send it
/// (docs/design/47-commitment-kernel.md).
///
/// # The design problem
///
/// This element sits between the user and every message they send, so the
/// failure mode is not "unclear" — it is "in the way". A strip that announces
/// itself on every keystroke gets ignored within a day, and an ignored safety
/// signal is worse than none, because the one time it says something important
/// nobody is looking.
///
/// So it is built to be *quiet in proportion to consequence*:
///
/// - **Ordinary work** — one dim line in micro-label type. Reads as chrome.
/// - **Consequential** (irreversible, deferred, audited, or a question worth
///   asking) — the accent appears and the reason is stated in words. The
///   accent is otherwise unused here, so its arrival means something.
/// - **Never** while the field is empty or the prompt is too short to read.
///
/// Expanding it gives the full inspector: every signal with the weight it
/// carried, and exactly what the run narrows. That is the same evidence
/// `vak intent explain` prints, because a reading nobody can inspect is a
/// reading nobody can argue with.
///
/// Resolution is free — deterministic tiers only, no dispatch — so this costs
/// nothing but a local round trip, and it never blocks sending.

import { For, Show, createEffect, createSignal, onCleanup } from "solid-js";

import { explainIntent, type IntentExplain } from "../api";
import Icon from "./Icon";

/// Long enough that a half-typed word does not thrash the endpoint, short
/// enough that the strip has settled before a fast typist reaches for Enter.
const DEBOUNCE_MS = 260;
/// Below this there is nothing to read, and guessing at two characters would
/// produce exactly the flickering nonsense that trains people to ignore it.
const MIN_CHARS = 12;

type Tone = "quiet" | "notice" | "warn";

/// How loudly this reading deserves to be presented.
///
/// Only three states, and the top one is genuinely rare. If everything is
/// important, nothing is.
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

/// The one line. Written as a claim about what will happen, not a label:
/// "8 tools" is inert, "approval will be asked" is actionable.
function summary(intent: IntentExplain): string {
  const { reading, engagement } = intent;
  const parts: string[] = [reading.act];
  if (engagement.posture.open_commitment) parts.push("opens a commitment");
  else if (engagement.posture.managed) parts.push("managed");
  const caps = engagement.limits.capabilities;
  if (caps.kind === "only") {
    parts.push(caps.names.length === 0 ? "no tools" : `${caps.names.length} tools`);
  }
  if (engagement.limits.approval_ceiling === "ask") parts.push("will ask before acting");
  if (engagement.posture.hil === "defer") parts.push("will queue questions");
  if (reading.evidence !== "none") parts.push(`${reading.evidence} evidence`);
  return parts.join(" · ");
}

export function IntentStrip(props: { prompt: string; disabled?: boolean }) {
  const [intent, setIntent] = createSignal<IntentExplain | null>(null);
  const [open, setOpen] = createSignal(false);

  createEffect(() => {
    const prompt = props.prompt.trim();
    if (props.disabled || prompt.length < MIN_CHARS) {
      setIntent(null);
      return;
    }
    const controller = new AbortController();
    const timer = setTimeout(() => {
      void explainIntent(prompt, controller.signal)
        .then(setIntent)
        // Silent: this is an optional read-out, and an error toast for a
        // background explain would be exactly the nagging it must avoid.
        .catch(() => {});
    }, DEBOUNCE_MS);
    onCleanup(() => {
      clearTimeout(timer);
      controller.abort();
    });
  });

  return (
    <Show when={intent()}>
      {(current) => (
        <div class={`intent-strip intent-${toneOf(current())}`}>
          <button
            class="intent-line"
            aria-expanded={open()}
            onClick={() => setOpen(!open())}
            title="How vak read this request, and what it will narrow"
          >
            <span class="intent-mark" aria-hidden="true" />
            <span class="intent-summary">{summary(current())}</span>
            <Show when={!current().provenance.reproducible}>
              <span class="intent-inferred" title="A model classified this; it is not reproducible from the ledger alone">
                inferred
              </span>
            </Show>
            <span class="intent-chev" classList={{ "intent-chev-open": open() }}>
              <Icon name="chevron" size={12} />
            </span>
          </button>

          {/* The consequential sentence is shown WITHOUT expanding, because
              the whole point of the warn tone is that it must not require a
              click to be seen. */}
          <Show when={toneOf(current()) === "warn" && current().engagement.posture.note}>
            {(note) => <p class="intent-note">{note().split("\n")[0]}</p>}
          </Show>

          <Show when={open()}>
            <div class="intent-detail">
              <dl class="intent-axes">
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
                    <div>
                      <dt>{k}</dt>
                      <dd>{v}</dd>
                    </div>
                  )}
                </For>
              </dl>

              <Show
                when={current().narrows.length > 0}
                fallback={
                  <p class="intent-empty">
                    Nothing narrowed — this runs with everything available.
                  </p>
                }
              >
                <ul class="intent-narrows">
                  <For each={current().narrows}>{(line) => <li>{line}</li>}</For>
                </ul>
              </Show>

              <details class="intent-signals">
                <summary>
                  {current().provenance.signals.length} signal
                  {current().provenance.signals.length === 1 ? "" : "s"} · resolved by{" "}
                  {current().provenance.tier}
                </summary>
                <ul>
                  <For each={current().provenance.signals}>
                    {(signal) => (
                      <li>
                        <span class="sig-kind">{signal.kind}</span>
                        <span class="sig-weight">{signal.weight.toFixed(2)}</span>
                        <span class="sig-detail">{signal.detail}</span>
                      </li>
                    )}
                  </For>
                </ul>
              </details>
            </div>
          </Show>
        </div>
      )}
    </Show>
  );
}
