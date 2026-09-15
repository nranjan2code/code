/// The commitment portfolio & intent kernel suite (docs/design/47-commitment-kernel.md).
///
/// Four questions, in the order an operator actually asks them:
///   1. What does this agent owe?
///   2. What will it do next and why?
///   3. What is stuck and on what?
///   4. Can I believe the ones it says are finished?
///
/// Rows, not cards, for the ledger. Rich 7-axis simulation for the intent kernel.
/// Zero markdown/emoji icons: styling is built from SVG status dots, tone chips,
/// and crisp typography.

import { For, Match, Show, Switch, createEffect, createMemo, createResource, createSignal } from "solid-js";

import { api } from "./api";
import { PageHeader } from "./display";
import { navigate, pushToast, route } from "./store";
import { timeAgo } from "./time";
import type {
  Commitment,
  CommitmentEpisode,
  CommitmentPriority,
  CriterionState,
  IntentExplain,
  IntentPolicy,
  Satisfaction,
  Verdict,
} from "./types";

const LATTICE: Satisfaction[] = ["asserted", "cited", "observed", "attested"];

const STRENGTH_BLURB: Record<Satisfaction, string> = {
  asserted: "the model said so",
  cited: "the model said so, with sources",
  observed: "the runtime checked it against the world",
  attested: "an independent party confirmed it",
};

function rank(strength: Satisfaction): number {
  return LATTICE.indexOf(strength);
}

/// The satisfaction lattice, drawn.
///
/// Four segments. Solid up to what was achieved, hollow beyond it. A tick
/// marks the level this work is actually held to. When achieved falls short,
/// the shortfall carries the accent — this is a live gap that blocks a
/// closure, which is exactly what the one accent colour is reserved for.
export function EvidenceMeter(props: {
  achieved: Satisfaction;
  required: Satisfaction;
  compact?: boolean;
}) {
  const achievedRank = () => rank(props.achieved);
  const requiredRank = () => rank(props.required);
  const short = () => achievedRank() < requiredRank();
  const label = () =>
    short()
      ? `Evidence ${props.achieved}; this work requires ${props.required}. Cannot close as fulfilled.`
      : `Evidence ${props.achieved}; requirement ${props.required} met.`;

  return (
    <div class="ev" role="img" aria-label={label()}>
      <div class="ev-track" classList={{ "ev-short": short() }}>
        <For each={LATTICE}>
          {(level, index) => (
            <span
              class="ev-seg"
              classList={{
                "ev-filled": index() <= achievedRank(),
                "ev-deficit": index() > achievedRank() && index() <= requiredRank(),
                "ev-required": index() === requiredRank(),
              }}
              title={`${level} — ${STRENGTH_BLURB[level]}`}
            />
          )}
        </For>
      </div>
      <Show when={!props.compact}>
        <span class="ev-label" classList={{ "ev-label-short": short() }}>
          {short() ? `${props.achieved} · needs ${props.required}` : props.achieved}
        </span>
      </Show>
    </div>
  );
}

/// Run-state dot, following the existing signature component: hue plus a fill
/// difference plus a spoken label, never hue alone.
function PhaseDot(props: { commitment: Commitment; held?: string | null }) {
  const kind = () => {
    if (props.commitment.phase === "closed") {
      return props.commitment.closure?.verdict === "fulfilled" ? "done" : "ended";
    }
    if (props.commitment.phase === "blocked") return "blocked";
    if (props.commitment.phase === "suspended") return "waiting";
    if (props.held) return "held";
    return "active";
  };
  const words: Record<string, string> = {
    done: "fulfilled",
    ended: `closed ${props.commitment.closure?.verdict ?? ""}`.trim(),
    blocked: "blocked",
    waiting: "waiting",
    held: "held",
    active: "active",
  };
  return <span class={`cdot cdot-${kind()}`} role="img" aria-label={words[kind()]} />;
}

function CriteriaBar(props: { commitment: Commitment }) {
  const total = () => props.commitment.criteria.length;
  const passed = () =>
    props.commitment.criteria.filter((c) => c.result?.kind === "passed").length;
  return (
    <Show
      when={total() > 0}
      fallback={
        <span class="crit-none" title="No criteria registered — nothing here can be machine-checked yet">
          none set
        </span>
      }
    >
      <span class="crit" aria-label={`${passed()} of ${total()} criteria passed`}>
        <For each={props.commitment.criteria}>
          {(criterion) => (
            <span
              class="crit-pip"
              classList={{
                "crit-pass": criterion.result?.kind === "passed",
                "crit-fail": criterion.result?.kind === "failed",
                "crit-unknown": criterion.result?.kind === "unknown",
              }}
              title={`${criterion.statement} — ${criterion.result?.kind ?? "not evaluated"}`}
            />
          )}
        </For>
        <span class="crit-count">
          {passed()}/{total()}
        </span>
      </span>
    </Show>
  );
}

/// Why this row sits where it does.
///
/// The scheduler is deterministic and every priority decomposes into named
/// components, so the portfolio shows the arithmetic rather than an opaque
/// rank. In a product whose thesis is auditability, "why did it pick that
/// one" must not be the single unanswerable question.
function WhyCell(props: { priority?: CommitmentPriority }) {
  const [open, setOpen] = createSignal(false);
  return (
    <Show when={props.priority} fallback={<span class="why-none">—</span>}>
      {(priority) => (
        <Show
          when={!priority().withheld}
          fallback={
            <span class="why-held" title={priority().withheld ?? ""}>
              {priority().withheld}
            </span>
          }
        >
          <button
            type="button"
            class="why"
            aria-expanded={open()}
            onClick={() => setOpen(!open())}
            title="How the scheduler ranked this"
          >
            <span class="why-score">{priority().score.toFixed(1)}</span>
            <Show when={open()}>
              <span class="why-parts">
                <For each={priority().components}>
                  {([name, value]) => (
                    <span class="why-part">
                      {name} <b>{value > 0 ? "+" : ""}{value.toFixed(1)}</b>
                    </span>
                  )}
                </For>
              </span>
            </Show>
          </button>
        </Show>
      )}
    </Show>
  );
}

const VERDICTS: Verdict[] = [
  "fulfilled",
  "partial",
  "failed",
  "abandoned",
  "expired",
  "unknown",
];

function CloseControl(props: { commitment: Commitment; onDone: () => void }) {
  const [open, setOpen] = createSignal(false);
  const [verdict, setVerdict] = createSignal<Verdict>("abandoned");
  const [note, setNote] = createSignal("");
  const [busy, setBusy] = createSignal(false);

  const close = async () => {
    setBusy(true);
    try {
      await api.closeCommitment(props.commitment.commitment_id, verdict(), note());
      pushToast("info", `Closed as ${verdict()}`);
      setOpen(false);
      props.onDone();
    } catch (error) {
      pushToast("alert", String(error instanceof Error ? error.message : error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Show
      when={open()}
      fallback={
        <button type="button" class="ghost small" onClick={() => setOpen(true)}>
          Close
        </button>
      }
    >
      <div class="close-form">
        <select
          value={verdict()}
          onChange={(e) => setVerdict(e.currentTarget.value as Verdict)}
          aria-label="Verdict"
        >
          <For each={VERDICTS}>{(v) => <option value={v}>{v}</option>}</For>
        </select>
        <input
          placeholder="Closing note / reason"
          value={note()}
          onInput={(e) => setNote(e.currentTarget.value)}
          aria-label="Closing note"
        />
        <button type="button" disabled={busy()} onClick={() => void close()}>
          {busy() ? "…" : "Confirm"}
        </button>
        <button type="button" class="ghost small" onClick={() => setOpen(false)}>
          Cancel
        </button>
      </div>
    </Show>
  );
}

function Row(props: {
  commitment: Commitment;
  priority?: CommitmentPriority;
  onChange: () => void;
}) {
  const [expanded, setExpanded] = createSignal(false);
  const achieved = (): Satisfaction => {
    const required = props.commitment.criteria.filter((c) => c.required);
    if (required.length === 0) return "asserted";
    return required
      .map((c) => c.strength ?? "asserted")
      .reduce((weakest, s) => (rank(s) < rank(weakest) ? s : weakest), "attested" as Satisfaction);
  };

  return (
    <>
      <tr
        class="crow"
        classList={{ dim: props.commitment.phase === "closed" }}
        onClick={() => setExpanded(!expanded())}
      >
        <td class="crow-state">
          <PhaseDot commitment={props.commitment} held={props.priority?.withheld} />
        </td>
        <td class="crow-objective">
          <span class="crow-title">{props.commitment.spec.objective}</span>
          <span class="crow-sub">
            <span class="chip chip-phrase" style={{ "font-size": "10px", "padding": "1px 6px" }}>
              {props.commitment.spec.reading.act}
            </span>
            {" · "}
            <span class="chip chip-phrase" style={{ "font-size": "10px", "padding": "1px 6px" }}>
              {props.commitment.spec.reading.horizon}
            </span>
            {" · "}
            <span class="chip chip-phrase" style={{ "font-size": "10px", "padding": "1px 6px" }}>
              {props.commitment.spec.reading.stakes}
            </span>
            <Show when={props.commitment.consecutive_stalls > 0}>
              {" · "}
              <span class="crow-stall">
                {props.commitment.consecutive_stalls} stalled
              </span>
            </Show>
          </span>
        </td>
        <td class="crow-evidence">
          <EvidenceMeter
            achieved={props.commitment.closure?.strength ?? achieved()}
            required={props.commitment.spec.min_satisfaction}
          />
        </td>
        <td class="crow-criteria">
          <CriteriaBar commitment={props.commitment} />
        </td>
        <td class="crow-spend num">${props.commitment.spend_usd.toFixed(3)}</td>
        <td class="crow-why">
          <WhyCell priority={props.priority} />
        </td>
        <td class="crow-age">{timeAgo(props.commitment.updated_at)}</td>
      </tr>
      <Show when={expanded()}>
        <tr class="crow-detail">
          <td colSpan={7}>
            <div class="cdetail">
              <div class="cdetail-facts">
                <div>
                  <span class="k">id</span>
                  <span class="v mono">{props.commitment.commitment_id}</span>
                </div>
                <div>
                  <span class="k">opened</span>
                  <span class="v">{timeAgo(props.commitment.opened_at)}</span>
                </div>
                <div>
                  <span class="k">episodes</span>
                  <span class="v">{props.commitment.episodes.length}</span>
                </div>
                <div>
                  <span class="k">min satisfaction</span>
                  <span class="v">{props.commitment.spec.min_satisfaction}</span>
                </div>
                <Show when={props.commitment.spec.economics.lifetime_budget_usd}>
                  <div>
                    <span class="k">budget cap</span>
                    <span class="v">${props.commitment.spec.economics.lifetime_budget_usd?.toFixed(2)}</span>
                  </div>
                </Show>
                <Show when={props.commitment.blocker}>
                  <div>
                    <span class="k">blocked</span>
                    <span class="v" style={{ "color": "var(--red)" }}>{props.commitment.blocker}</span>
                  </div>
                </Show>
                <Show when={props.commitment.drift.length > 0}>
                  <div>
                    <span class="k">drift alerts</span>
                    <span class="v" style={{ "color": "var(--yellow)" }}>{props.commitment.drift.join("; ")}</span>
                  </div>
                </Show>
              </div>

              {/* Criteria List */}
              <Show when={props.commitment.criteria.length > 0}>
                <div style={{ "margin-top": "14px" }}>
                  <span class="eyebrow">Satisfaction Criteria ({props.commitment.criteria.length})</span>
                  <ul class="cdetail-criteria">
                    <For each={props.commitment.criteria}>
                      {(criterion: CriterionState) => (
                        <li>
                          <span
                            class="crit-pip"
                            classList={{
                              "crit-pass": criterion.result?.kind === "passed",
                              "crit-fail": criterion.result?.kind === "failed",
                              "crit-unknown": criterion.result?.kind === "unknown",
                            }}
                          />
                          <span class="cdetail-stmt">
                            <strong>{criterion.statement}</strong>
                            <Show when={criterion.result?.kind === "passed"}>
                              <span class="dim" style={{ "display": "block", "font-size": "11px", "margin-top": "2px" }}>
                                Evidence: {(criterion.result as { kind: "passed"; evidence: string }).evidence}
                              </span>
                            </Show>
                            <Show when={criterion.result && ("reason" in criterion.result)}>
                              <span class="dim" style={{ "display": "block", "font-size": "11px", "margin-top": "2px", "color": "var(--red)" }}>
                                Failure: {(criterion.result as { reason: string }).reason}
                              </span>
                            </Show>
                          </span>
                          <Show when={criterion.strength}>
                            <span class="cdetail-strength chip chip-phrase">{criterion.strength}</span>
                          </Show>
                        </li>
                      )}
                    </For>
                  </ul>
                </div>
              </Show>

              {/* Episodes Timeline */}
              <Show when={props.commitment.episodes.length > 0}>
                <div style={{ "margin-top": "14px" }}>
                  <span class="eyebrow">Work Episode History ({props.commitment.episodes.length})</span>
                  <div class="episode-timeline">
                    <For each={props.commitment.episodes}>
                      {(ep: CommitmentEpisode) => (
                        <div class="episode-item">
                          <span class="mono" style={{ "font-size": "11px" }}>{ep.episode_id.slice(0, 8)}</span>
                          <a
                            href={`#/sessions/${ep.session_id}`}
                            class="mono dim"
                            style={{ "text-decoration": "underline" }}
                            title="Inspect Session Transcript"
                          >
                            session:{ep.session_id.slice(0, 8)}
                          </a>
                          <span class="dim">{timeAgo(ep.started_at)}</span>
                          <span>
                            <span class={`advancement-badge ${ep.advancement?.kind ?? "advanced"}`}>
                              {ep.advancement?.kind ?? "completed"}
                            </span>
                          </span>
                          <span class="num">${ep.spend_usd.toFixed(3)}</span>
                        </div>
                      )}
                    </For>
                  </div>
                </div>
              </Show>

              {/* Closure Controls / Status */}
              <Show
                when={props.commitment.closure}
                fallback={
                  <div class="cdetail-actions">
                    <CloseControl commitment={props.commitment} onDone={props.onChange} />
                  </div>
                }
              >
                {(closure) => (
                  <div class="cdetail-closure">
                    closed <strong style={{ "color": "var(--text)" }}>{closure().verdict}</strong> on{" "}
                    <span class="chip chip-phrase">{closure().strength}</span> evidence
                    {closure().note ? ` — ${closure().note}` : ""}
                  </div>
                )}
              </Show>
            </div>
          </td>
        </tr>
      </Show>
    </>
  );
}

const PRESET_INTENT_PROMPTS = [
  {
    title: "Code Bug Fix & Test",
    prompt: "Fix the race condition in the worker pool and run all cargo integration tests.",
  },
  {
    title: "Direct Query",
    prompt: "What is the capital of France and what is the current local time there?",
  },
  {
    title: "Database Refactoring",
    prompt: "Migrate the billing schema to add support for multiple currencies, with backward compatibility.",
  },
  {
    title: "Security Audit",
    prompt: "Inspect system authentication logs for failed SSH logins and generate a summary report.",
  },
  {
    title: "Executive Briefing",
    prompt: "Draft a quarterly executive briefing for leadership on platform stability and token burn.",
  },
];

function IntentSimulator() {
  const [testPrompt, setTestPrompt] = createSignal(PRESET_INTENT_PROMPTS[0].prompt);
  const [debouncedPrompt, setDebouncedPrompt] = createSignal(PRESET_INTENT_PROMPTS[0].prompt);
  const [surface, setSurface] = createSignal("");

  let timer: ReturnType<typeof setTimeout> | undefined;
  createEffect(() => {
    const p = testPrompt();
    clearTimeout(timer);
    timer = setTimeout(() => {
      setDebouncedPrompt(p.trim());
    }, 250);
  });

  const [explain] = createResource(
    () => ({ prompt: debouncedPrompt(), surface: surface() || undefined }),
    ({ prompt, surface: s }) => {
      if (!prompt) return null;
      return api.explainIntent(prompt, s);
    },
  );

  return (
    <div class="intent-sim-container">
      <div class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Intent Kernel & 7-Axis Classifier</h2>
            <p class="dim">
              The commitment kernel analyzes work requests across 7 orthogonal axes before any turn runs,
              determining capability slicing, commitment promotion, and satisfaction standards.
            </p>
          </div>
        </div>

        {/* Presets */}
        <div class="eyebrow" style={{ "margin-bottom": "6px" }}>Sample Request Presets</div>
        <div class="intent-presets-grid">
          <For each={PRESET_INTENT_PROMPTS}>
            {(preset) => (
              <button
                type="button"
                class="intent-preset-btn"
                classList={{ active: testPrompt() === preset.prompt }}
                onClick={() => setTestPrompt(preset.prompt)}
              >
                <span class="intent-preset-title">{preset.title}</span>
                <span class="intent-preset-prompt">{preset.prompt}</span>
              </button>
            )}
          </For>
        </div>

        {/* Custom Input */}
        <div class="form-row" style={{ "margin-top": "14px" }}>
          <label>Test Prompt</label>
          <textarea
            rows={2}
            value={testPrompt()}
            onInput={(e) => setTestPrompt(e.currentTarget.value)}
            placeholder="Type any instruction to test intent classification..."
            style={{ "width": "100%", "font-family": "var(--font-sans)", "font-size": "13px" }}
          />
        </div>
      </div>

      <Show when={explain.loading}>
        <div class="skeleton-rows">
          <div class="skeleton-row" />
          <div class="skeleton-row" />
        </div>
      </Show>

      <Show when={explain()}>
        {(exp: () => IntentExplain) => (
          <>
            {/* 7-Axes Radar Deck */}
            <div class="panel">
              <div class="panel-title-row">
                <div>
                  <h3>7-Axis Behavioral Reading</h3>
                  <p class="dim">Inferred work characteristics driving runtime subsystems.</p>
                </div>
                <span class="chip chip-tone-success">zero-cost classification</span>
              </div>

              <div class="intent-axes-grid">
                <div class="axis-card act">
                  <div class="axis-header">
                    <span class="axis-name">Act</span>
                  </div>
                  <span class="axis-val">{exp().reading.act}</span>
                  <span class="axis-desc">Drives capability slice & stop profile</span>
                </div>

                <div class="axis-card horizon">
                  <div class="axis-header">
                    <span class="axis-name">Horizon</span>
                  </div>
                  <span class="axis-val">{exp().reading.horizon}</span>
                  <span class="axis-desc">Drives managed admission & commitment creation</span>
                </div>

                <div class="axis-card stakes">
                  <div class="axis-header">
                    <span class="axis-name">Stakes</span>
                  </div>
                  <span class="axis-val">{exp().reading.stakes}</span>
                  <span class="axis-desc">Drives approval ceiling & checkpoint requirement</span>
                </div>

                <div class="axis-card evidence">
                  <div class="axis-header">
                    <span class="axis-name">Evidence</span>
                  </div>
                  <span class="axis-val">{exp().reading.evidence}</span>
                  <span class="axis-desc">Minimum satisfaction strength to close</span>
                </div>

                <div class="axis-card clarity">
                  <div class="axis-header">
                    <span class="axis-name">Clarity</span>
                  </div>
                  <span class="axis-val">{exp().reading.clarity}</span>
                  <span class="axis-desc">Ask vs. state-an-assumption rule</span>
                </div>

                <div class="axis-card modality">
                  <div class="axis-header">
                    <span class="axis-name">Modality</span>
                  </div>
                  <span class="axis-val">{exp().engagement.limits.required_modalities.join(", ") || "text"}</span>
                  <span class="axis-desc">Ladder model filtering & format</span>
                </div>

                <div class="axis-card attendance">
                  <div class="axis-header">
                    <span class="axis-name">Attendance</span>
                  </div>
                  <span class="axis-val">{exp().reading.attendance}</span>
                  <span class="axis-desc">HIL mode & delivery cadence</span>
                </div>
              </div>
            </div>

            {/* Derived Engagement & Narrowing */}
            <div class="intent-engagement-grid">
              <div class="panel">
                <div class="panel-title-row">
                  <div>
                    <h3>Derived Engagement Posture</h3>
                    <p class="dim">Safety boundaries calculated from the 7 axes.</p>
                  </div>
                </div>
                <div style={{ "display": "grid", "grid-template-columns": "140px 1fr", "gap": "10px", "font-size": "12px" }}>
                  <span class="dim">Managed Mode:</span>
                  <span><strong>{exp().engagement.posture.managed ? "Yes (Durable Session)" : "No (Direct)"}</strong></span>

                  <span class="dim">Open Commitment:</span>
                  <span><strong>{exp().engagement.posture.open_commitment ? "Yes" : "No"}</strong></span>

                  <span class="dim">Approval Ceiling:</span>
                  <span><span class="chip chip-phrase">{exp().engagement.limits.approval_ceiling}</span></span>

                  <span class="dim">Permission Ceiling:</span>
                  <span><span class="chip chip-phrase">{exp().engagement.limits.permission_ceiling}</span></span>

                  <span class="dim">Stop Condition:</span>
                  <span><code>{exp().engagement.posture.stop}</code></span>

                  <span class="dim">Context Profile:</span>
                  <span><code>{exp().engagement.posture.context}</code></span>

                  <span class="dim">Delivery Cadence:</span>
                  <span>{exp().engagement.posture.delivery.cadence} ({exp().engagement.posture.delivery.urgency})</span>
                </div>
              </div>

              <div class="panel">
                <div class="panel-title-row">
                  <div>
                    <h3>Progressive Capability Slicing</h3>
                    <p class="dim">What this request narrows versus doing nothing.</p>
                  </div>
                </div>

                <Show
                  when={exp().narrows.length > 0}
                  fallback={<p class="dim">No additional restrictions applied beyond default posture.</p>}
                >
                  <div class="narrowing-list">
                    <For each={exp().narrows}>
                      {(narrow) => (
                        <div class="narrowing-item">
                          <span class="narrowing-bullet">·</span>
                          <span>{narrow}</span>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>

                <Show when={exp().model_visible}>
                  <div style={{ "margin-top": "12px" }}>
                    <span class="eyebrow">Model Visible Framing</span>
                    <pre style={{ "background": "var(--surface-raised)", "padding": "8px 10px", "border-radius": "4px", "font-size": "11px", "white-space": "pre-wrap", "margin": "4px 0 0" }}>
                      {exp().model_visible}
                    </pre>
                  </div>
                </Show>
              </div>
            </div>
          </>
        )}
      </Show>
    </div>
  );
}

function KernelPolicyView() {
  const [policy] = createResource(() => api.intentPolicy());

  return (
    <div class="policy-grid">
      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Intent Kernel Configuration</h2>
            <p class="dim">Governing prompt reading, classification confidence, and autonomy.</p>
          </div>
          <Show when={policy()}>
            {(p) => (
              <span class={`chip ${p().intent.enabled ? "chip-tone-success" : "chip-tone-warning"}`}>
                {p().intent.enabled ? "enabled" : "disabled"}
              </span>
            )}
          </Show>
        </div>

        <Show when={policy()} fallback={<div class="skel skel-block" />}>
          {(p: () => IntentPolicy) => (
            <div style={{ "display": "grid", "grid-template-columns": "180px 1fr", "gap": "10px", "font-size": "12px" }}>
              <span class="dim">Capability Slicing:</span>
              <span>{p().intent.slice_capabilities ? "Active (Progressive Disclosure)" : "Inactive"}</span>

              <span class="dim">Accept Confidence:</span>
              <span class="num">{(p().intent.accept_confidence * 100).toFixed(0)}%</span>

              <span class="dim">Provisional Confidence:</span>
              <span class="num">{(p().intent.provisional_confidence * 100).toFixed(0)}%</span>

              <span class="dim">Autonomy Level:</span>
              <span><span class="chip chip-phrase">{p().intent.autonomy}</span></span>

              <span class="dim">Max Classification Budget:</span>
              <span class="num">${p().intent.max_classify_usd.toFixed(3)}</span>

              <span class="dim">Escalation Strategy:</span>
              <span><code>{p().intent.escalate}</code></span>
            </div>
          )}
        </Show>
      </section>

      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Commitment Engine Constraints</h2>
            <p class="dim">Rules governing commitment lifecycle, budgets, and stall detection.</p>
          </div>
          <Show when={policy()}>
            {(p) => (
              <span class={`chip ${p().commitment.enabled ? "chip-tone-success" : "chip-tone-warning"}`}>
                {p().commitment.enabled ? "enabled" : "disabled"}
              </span>
            )}
          </Show>
        </div>

        <Show when={policy()} fallback={<div class="skel skel-block" />}>
          {(p: () => IntentPolicy) => (
            <div style={{ "display": "grid", "grid-template-columns": "180px 1fr", "gap": "10px", "font-size": "12px" }}>
              <span class="dim">Consecutive Stall Limit:</span>
              <span><strong>{p().commitment.stall_limit} episodes</strong></span>

              <span class="dim">Review Cadence:</span>
              <span>{p().commitment.review_every_hours ? `Every ${p().commitment.review_every_hours}h` : "On each episode"}</span>

              <span class="dim">Default Time-To-Live:</span>
              <span>{p().commitment.default_ttl_days ? `${p().commitment.default_ttl_days} days` : "Unlimited"}</span>

              <span class="dim">Lifetime Budget Cap:</span>
              <span>{p().commitment.lifetime_budget_usd ? `$${p().commitment.lifetime_budget_usd!.toFixed(2)}` : "No limit set"}</span>
            </div>
          )}
        </Show>

        <div style={{ "margin-top": "16px", "padding-top": "12px", "border-top": "1px solid var(--border-soft)" }}>
          <span class="eyebrow">Satisfaction Lattice Standards</span>
          <div style={{ "display": "flex", "flex-direction": "column", "gap": "6px", "margin-top": "6px", "font-size": "11.5px" }}>
            <div><strong style={{ "color": "var(--text)" }}>1. Asserted:</strong> The model said so on its own say-so.</div>
            <div><strong style={{ "color": "var(--text)" }}>2. Cited:</strong> The model said so, accompanied by verified citations.</div>
            <div><strong style={{ "color": "var(--text)" }}>3. Observed:</strong> The runtime checked the fact against host reality.</div>
            <div><strong style={{ "color": "var(--text)" }}>4. Attested:</strong> An independent external party or test affirmed it.</div>
          </div>
        </div>
      </section>
    </div>
  );
}

export function Commitments() {
  type CommTab = "portfolio" | "intent" | "policy";
  const [activeTab, setActiveTab] = createSignal<CommTab>("portfolio");

  createEffect(() => {
    const r = route().split("?", 1)[0];
    if (r === "#/commitments/intent") setActiveTab("intent");
    else if (r === "#/commitments/policy") setActiveTab("policy");
    else if (r === "#/commitments/portfolio" || r === "#/commitments") setActiveTab("portfolio");
  });

  const switchTab = (tab: CommTab) => {
    setActiveTab(tab);
    navigate(`#/commitments/${tab}`);
  };

  const [phaseFilter, setPhaseFilter] = createSignal<string>("all");
  const [evidenceFilter, setEvidenceFilter] = createSignal<string>("all");
  const [searchQuery, setSearchQuery] = createSignal<string>("");

  const [data, { refetch }] = createResource(() => api.commitments(true));

  const priorityFor = (id: string) =>
    data()?.priorities.find((p) => p.commitment_id === id);

  // Filtered Commitments
  const filteredCommitments = createMemo(() => {
    const list = data()?.commitments ?? [];
    const pf = phaseFilter();
    const ef = evidenceFilter();
    const q = searchQuery().trim().toLowerCase();

    return list.filter((c) => {
      // Phase match
      if (pf !== "all") {
        if (pf === "active" && c.phase !== "active") return false;
        if (pf === "blocked" && c.phase !== "blocked") return false;
        if (pf === "suspended" && c.phase !== "suspended") return false;
        if (pf === "closed" && c.phase !== "closed") return false;
      }
      // Evidence match
      if (ef !== "all") {
        if (c.spec.min_satisfaction !== ef) return false;
      }
      // Search match
      if (q) {
        const matchesObj = c.spec.objective.toLowerCase().includes(q);
        const matchesId = c.commitment_id.toLowerCase().includes(q);
        const matchesCrit = c.criteria.some((cr) => cr.statement.toLowerCase().includes(q));
        if (!matchesObj && !matchesId && !matchesCrit) return false;
      }
      return true;
    });
  });

  // Posture Metrics
  const metrics = createMemo(() => {
    const list = data()?.commitments ?? [];
    const active = list.filter((c) => c.phase === "active").length;
    const blocked = list.filter((c) => c.phase === "blocked").length;
    const suspended = list.filter((c) => c.phase === "suspended").length;
    const closed = list.filter((c) => c.phase === "closed").length;
    const totalSpend = list.reduce((acc, c) => acc + c.spend_usd, 0);

    const metCount = list.filter((c) => {
      const achieved = c.closure?.strength ?? "asserted";
      return rank(achieved) >= rank(c.spec.min_satisfaction);
    }).length;
    const metRatio = list.length > 0 ? Math.round((metCount / list.length) * 100) : 100;

    return { active, blocked, suspended, closed, totalSpend, metRatio };
  });

  return (
    <div class="view">
      <PageHeader
        title="Commitments & Intent Kernel"
        description="Durable obligations, 7-axis behavioral readings, deterministic scheduling arithmetic, and satisfaction lattice proof."
      />

      {/* Posture Masthead */}
      <div class="commitments-posture-deck">
        <div class="stat-card">
          <span class="stat-card-label">Active Obligations</span>
          <div style={{ "display": "flex", "align-items": "center", "gap": "6px", "margin": "4px 0" }}>
            <strong style={{ "font-size": "18px", "font-weight": "700" }}>
              {metrics().active}
            </strong>
            <Show when={metrics().blocked > 0}>
              <span class="chip chip-tone-danger">{metrics().blocked} blocked</span>
            </Show>
            <Show when={metrics().suspended > 0}>
              <span class="chip chip-tone-warning">{metrics().suspended} waiting</span>
            </Show>
          </div>
          <span class="stat-card-hint">
            {metrics().closed} closed / historical
          </span>
        </div>

        <div class="stat-card">
          <span class="stat-card-label">Evidence Satisfaction</span>
          <div style={{ "display": "flex", "align-items": "center", "gap": "6px", "margin": "4px 0" }}>
            <strong style={{ "font-size": "18px", "font-weight": "700" }}>
              {metrics().metRatio}%
            </strong>
            <span class="chip chip-tone-success">standards met</span>
          </div>
          <span class="stat-card-hint">Lattice verified</span>
        </div>

        <div class="stat-card">
          <span class="stat-card-label">Incurred Portfolio Spend</span>
          <div style={{ "display": "flex", "align-items": "center", "gap": "6px", "margin": "4px 0" }}>
            <strong style={{ "font-size": "18px", "font-weight": "700" }}>
              ${metrics().totalSpend.toFixed(3)}
            </strong>
          </div>
          <span class="stat-card-hint">Total episodes spend</span>
        </div>

        <div class="stat-card">
          <span class="stat-card-label">Intent Classification</span>
          <div style={{ "display": "flex", "align-items": "center", "gap": "6px", "margin": "4px 0" }}>
            <span class="chip chip-tone-info">7-Axis Kernel Active</span>
          </div>
          <span class="stat-card-hint">Progressive capability slicing</span>
        </div>
      </div>

      {/* Subnav Tabs */}
      <div class="commitments-tab-bar">
        <button
          type="button"
          class="commitments-tab-btn"
          classList={{ active: activeTab() === "portfolio" }}
          onClick={() => switchTab("portfolio")}
        >
          Active Portfolio
          <Show when={data()?.commitments.length}>
            <span class="nav-badge" style={{ "margin-left": "4px" }}>{data()?.commitments.length}</span>
          </Show>
        </button>
        <button
          type="button"
          class="commitments-tab-btn"
          classList={{ active: activeTab() === "intent" }}
          onClick={() => switchTab("intent")}
        >
          Intent Kernel & 7-Axis Simulator
        </button>
        <button
          type="button"
          class="commitments-tab-btn"
          classList={{ active: activeTab() === "policy" }}
          onClick={() => switchTab("policy")}
        >
          Kernel Policy & Standards
        </button>
      </div>

      {/* Tab 1: Active Portfolio */}
      <Show when={activeTab() === "portfolio"}>
        <div class="commitments-filter-toolbar">
          <input
            placeholder="Search commitments by objective, id, criteria..."
            value={searchQuery()}
            onInput={(e) => setSearchQuery(e.currentTarget.value)}
            style={{ "min-width": "260px", "flex": "1" }}
          />

          <div class="chips">
            <button
              class="chip-btn"
              classList={{ active: phaseFilter() === "all" }}
              onClick={() => setPhaseFilter("all")}
            >
              All Phases
            </button>
            <button
              class="chip-btn"
              classList={{ active: phaseFilter() === "active" }}
              onClick={() => setPhaseFilter("active")}
            >
              Active
            </button>
            <button
              class="chip-btn"
              classList={{ active: phaseFilter() === "blocked" }}
              onClick={() => setPhaseFilter("blocked")}
            >
              Blocked
            </button>
            <button
              class="chip-btn"
              classList={{ active: phaseFilter() === "suspended" }}
              onClick={() => setPhaseFilter("suspended")}
            >
              Suspended
            </button>
            <button
              class="chip-btn"
              classList={{ active: phaseFilter() === "closed" }}
              onClick={() => setPhaseFilter("closed")}
            >
              Closed
            </button>
          </div>

          <div class="chips">
            <button
              class="chip-btn"
              classList={{ active: evidenceFilter() === "all" }}
              onClick={() => setEvidenceFilter("all")}
            >
              All Levels
            </button>
            <For each={LATTICE}>
              {(l) => (
                <button
                  class="chip-btn"
                  classList={{ active: evidenceFilter() === l }}
                  onClick={() => setEvidenceFilter(l)}
                >
                  {l}
                </button>
              )}
            </For>
          </div>

          <button type="button" class="ghost" onClick={() => void refetch()}>
            Refresh
          </button>
        </div>

        <Switch>
          <Match when={data.loading}>
            <div class="skeleton-rows" aria-busy="true" aria-label="Loading commitments">
              <For each={[0, 1, 2]}>{() => <div class="skeleton-row" />}</For>
            </div>
          </Match>
          <Match when={data.error}>
            <div class="empty empty-teach">
              <strong>Could not load commitments.</strong>
              <p>{String(data.error)}</p>
            </div>
          </Match>
          <Match when={(data()?.commitments.length ?? 0) === 0}>
            <div class="empty empty-teach">
              <strong>No commitments open.</strong>
              <p>
                vak opens one when a request reads as lasting beyond this session —
                something recurring, or work with a done-condition worth checking
                later. Short tasks never create one, so an empty list here usually
                means everything asked of it so far was finishable in the moment.
              </p>
            </div>
          </Match>
          <Match when={filteredCommitments().length === 0}>
            <div class="empty">
              No commitments match your active phase, evidence, or search filters.
            </div>
          </Match>
          <Match when={filteredCommitments().length > 0}>
            <table class="ctable">
              <thead>
                <tr>
                  <th class="sr-only">State</th>
                  <th>Objective & 7-Axis Reading</th>
                  <th>Evidence (Achieved vs Required)</th>
                  <th>Criteria</th>
                  <th class="num">Spend</th>
                  <th>Priority (Why)</th>
                  <th>Updated</th>
                </tr>
              </thead>
              <tbody>
                <For each={filteredCommitments()}>
                  {(commitment) => (
                    <Row
                      commitment={commitment}
                      priority={priorityFor(commitment.commitment_id)}
                      onChange={() => void refetch()}
                    />
                  )}
                </For>
              </tbody>
            </table>
          </Match>
        </Switch>
      </Show>

      {/* Tab 2: Intent Kernel & 7-Axis Simulator */}
      <Show when={activeTab() === "intent"}>
        <IntentSimulator />
      </Show>

      {/* Tab 3: Kernel Policy & Standards */}
      <Show when={activeTab() === "policy"}>
        <KernelPolicyView />
      </Show>
    </div>
  );
}
