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
import { mapAdminAgents, navigate, pushToast, route, selectedAgentId, selectedAgentIdOrUndefined } from "./store";
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
      await api.closeCommitment(props.commitment.commitment_id, verdict(), note(), props.commitment.admin_agent_id ?? selectedAgentIdOrUndefined());
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
            <span class="chip chip-phrase" style={{ "font-size": "12px", "padding": "1px 6px" }}>
              {props.commitment.spec.reading.act}
            </span>
            {" · "}
            <span class="chip chip-phrase" style={{ "font-size": "12px", "padding": "1px 6px" }}>
              {props.commitment.spec.reading.horizon}
            </span>
            {" · "}
            <span class="chip chip-phrase" style={{ "font-size": "12px", "padding": "1px 6px" }}>
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
                              <span class="dim" style={{ "display": "block", "font-size": "12px", "margin-top": "2px" }}>
                                Evidence: {(criterion.result as { kind: "passed"; evidence: string }).evidence}
                              </span>
                            </Show>
                            <Show when={criterion.result && ("reason" in criterion.result)}>
                              <span class="dim" style={{ "display": "block", "font-size": "12px", "margin-top": "2px", "color": "var(--red)" }}>
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
                          <span class="mono" style={{ "font-size": "12px" }}>{ep.episode_id.slice(0, 8)}</span>
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
    id: "code-fix",
    icon: "code",
    category: "Engineering",
    title: "Code Bug Fix & Test",
    summary: "Worker pool race condition & cargo test suite",
    prompt: "Fix the race condition in the worker pool and run all cargo integration tests.",
  },
  {
    id: "direct-query",
    icon: "chat",
    category: "Direct Query",
    title: "Direct Query",
    summary: "Capital of France & local time query",
    prompt: "What is the capital of France and what is the current local time there?",
  },
  {
    id: "db-refactor",
    icon: "database",
    category: "Long Horizon",
    title: "Database Refactoring",
    summary: "Multi-currency billing schema with backward compatibility",
    prompt: "Migrate the billing schema to add support for multiple currencies, with backward compatibility.",
  },
  {
    id: "security-audit",
    icon: "shield",
    category: "Governance",
    title: "Security Audit",
    summary: "Auth logs inspection for failed SSH logins",
    prompt: "Inspect system authentication logs for failed SSH logins and generate a summary report.",
  },
  {
    id: "exec-briefing",
    icon: "chart",
    category: "Synthesis",
    title: "Executive Briefing",
    summary: "Quarterly stability & token burn leadership briefing",
    prompt: "Draft a quarterly executive briefing for leadership on platform stability and token burn.",
  },
];

const DELIVERY_SURFACES = [
  { id: "", label: "Default (CLI / Local Interactive)" },
  { id: "telegram", label: "Telegram Bridge" },
  { id: "discord", label: "Discord Bridge" },
  { id: "slack", label: "Slack Workspace Bridge" },
  { id: "web_chat", label: "Web Chat Console" },
  { id: "daemon", label: "Automated Daemon / Watchdog" },
];

function renderPresetSvg(icon: string) {
  switch (icon) {
    case "code":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="preset-icon">
          <polyline points="16 18 22 12 16 6" />
          <polyline points="8 6 2 12 8 18" />
        </svg>
      );
    case "chat":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="preset-icon">
          <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
        </svg>
      );
    case "database":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="preset-icon">
          <ellipse cx="12" cy="5" rx="9" ry="3" />
          <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3" />
          <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5" />
        </svg>
      );
    case "shield":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="preset-icon">
          <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
        </svg>
      );
    case "chart":
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="preset-icon">
          <line x1="18" y1="20" x2="18" y2="10" />
          <line x1="12" y1="20" x2="12" y2="4" />
          <line x1="6" y1="20" x2="6" y2="14" />
        </svg>
      );
    default:
      return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="preset-icon">
          <circle cx="12" cy="12" r="10" />
        </svg>
      );
  }
}

function IntentSimulator() {
  const [testPrompt, setTestPrompt] = createSignal(PRESET_INTENT_PROMPTS[0].prompt);
  const [debouncedPrompt, setDebouncedPrompt] = createSignal(PRESET_INTENT_PROMPTS[0].prompt);
  const [surface, setSurface] = createSignal("");
  const [showSignals, setShowSignals] = createSignal(false);
  const [copiedFraming, setCopiedFraming] = createSignal(false);

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

  const copyFraming = (text: string) => {
    navigator.clipboard.writeText(text);
    setCopiedFraming(true);
    setTimeout(() => setCopiedFraming(false), 2000);
  };

  const horizonRank = (h: string) => {
    switch (h) {
      case "immediate": return 1;
      case "turn": return 2;
      case "session": return 3;
      case "durable": return 4;
      default: return 2;
    }
  };

  const stakesRank = (s: string) => {
    switch (s) {
      case "inert": return 1;
      case "reversible": return 2;
      case "costly": return 3;
      case "irreversible": return 4;
      default: return 1;
    }
  };

  const evidenceRank = (e: string) => {
    switch (e) {
      case "none": return 0;
      case "cited": return 1;
      case "verified":
      case "observed": return 2;
      case "audited":
      case "attested": return 3;
      default: return 0;
    }
  };

  return (
    <div class="intent-sim-container">
      {/* Top Controller Panel */}
      <div class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Intent Kernel & 7-Axis Classifier</h2>
            <p class="dim">
              The commitment kernel analyzes work requests across 7 orthogonal axes before any turn runs,
              determining capability slicing, commitment promotion, and satisfaction standards (docs/design/47).
            </p>
          </div>
        </div>

        {/* Presets Grid */}
        <div class="eyebrow" style={{ "margin-bottom": "8px" }}>Interactive Work Request Presets</div>
        <div class="intent-presets-grid">
          <For each={PRESET_INTENT_PROMPTS}>
            {(preset) => (
              <button
                type="button"
                class="intent-preset-card"
                classList={{ active: testPrompt() === preset.prompt }}
                onClick={() => setTestPrompt(preset.prompt)}
              >
                <div class="intent-preset-top">
                  <span class="intent-preset-icon-wrap">{renderPresetSvg(preset.icon)}</span>
                  <span class="intent-preset-category">{preset.category}</span>
                </div>
                <div class="intent-preset-title" title={preset.title}>{preset.title}</div>
                <div class="intent-preset-summary" title={preset.summary}>{preset.summary}</div>
              </button>
            )}
          </For>
        </div>

        {/* Custom Input & Surface Bar */}
        <div class="intent-input-section" style={{ "margin-top": "16px" }}>
          <div class="intent-input-header">
            <label class="font-semibold" style={{ "font-size": "12.5px" }}>Test Work Instruction Prompt</label>
            <div class="intent-surface-picker">
              <span class="dim" style={{ "font-size": "11.5px" }}>Delivery Surface:</span>
              <select
                value={surface()}
                onChange={(e) => setSurface(e.currentTarget.value)}
                class="intent-surface-select"
              >
                <For each={DELIVERY_SURFACES}>
                  {(s) => <option value={s.id}>{s.label}</option>}
                </For>
              </select>
            </div>
          </div>
          <div class="textarea-wrap" style={{ "position": "relative", "margin-top": "6px" }}>
            <textarea
              rows={3}
              value={testPrompt()}
              onInput={(e) => setTestPrompt(e.currentTarget.value)}
              placeholder="Type any work request to evaluate 7-axis intent classification..."
              class="intent-test-textarea"
            />
            <Show when={testPrompt()}>
              <button
                type="button"
                class="button ghost small clear-input-btn"
                onClick={() => setTestPrompt("")}
                title="Clear input"
              >
                Clear
              </button>
            </Show>
          </div>
        </div>
      </div>

      <Show when={explain.loading}>
        <div class="skeleton-rows" style={{ "margin": "16px 0" }}>
          <div class="skeleton-row" style={{ "height": "120px" }} />
          <div class="skeleton-row" style={{ "height": "80px" }} />
        </div>
      </Show>

      <Show when={explain()}>
        {(exp: () => IntentExplain) => (
          <>
            {/* 7-Axes Behavioral Reading Deck */}
            <div class="panel">
              <div class="panel-title-row">
                <div>
                  <h3>7-Axis Behavioral Reading</h3>
                  <p class="dim">Inferred orthogonal characteristics governing all downstream runtime subsystems.</p>
                </div>
                <div style={{ "display": "flex", "align-items": "center", "gap": "8px" }}>
                  <span class="chip chip-tone-success">
                    {exp().provenance.tier === "signals"
                      ? "⚡ zero-cost signals (tier-1)"
                      : `${exp().provenance.tier} (tier-${exp().provenance.tier === "declared" ? 0 : 2})`}
                  </span>
                  <button
                    type="button"
                    class="button ghost small"
                    onClick={() => setShowSignals(!showSignals())}
                  >
                    {showSignals() ? "Hide Evidence" : `Inference Signals (${exp().provenance.signals?.length ?? 0})`}
                  </button>
                </div>
              </div>

              {/* Collapsible Signals Evidence */}
              <Show when={showSignals()}>
                <div class="signals-evidence-drawer">
                  <div class="signals-header">
                    <span class="eyebrow">Classifier Provenance &amp; Signals</span>
                    <span class="dim small mono">
                      Resolver v{exp().provenance.resolver_version} ·{" "}
                      {exp().provenance.reproducible ? "Deterministic Replay Verified" : "Model-Assisted Inference"}
                    </span>
                  </div>
                  <Show
                    when={(exp().provenance.signals ?? []).length > 0}
                    fallback={<div class="dim small">No specific lexical signals triggered; fell back to default baseline posture.</div>}
                  >
                    <div class="signals-list">
                      <For each={exp().provenance.signals}>
                        {(sig) => (
                          <div class="signal-item">
                            <div class="signal-left">
                              <span class="signal-bullet">◈</span>
                              <strong class="signal-name">{sig.name}</strong>
                              <span class="signal-kind chip mono">{sig.kind}</span>
                            </div>
                            <div class="signal-right">
                              <span class="signal-weight mono">w: {sig.weight.toFixed(2)}</span>
                              <span class="signal-detail dim">{sig.detail}</span>
                            </div>
                          </div>
                        )}
                      </For>
                    </div>
                  </Show>
                </div>
              </Show>

              {/* The 7 Cards Grid */}
              <div class="intent-axes-grid">
                {/* 1. ACT */}
                <div class="axis-card act">
                  <div class="axis-header">
                    <span class="axis-name">1. Act</span>
                    <span class="axis-tag">Categorical</span>
                  </div>
                  <div class="axis-val-row">
                    <span class="axis-val">{exp().reading.act}</span>
                  </div>
                  <span class="axis-desc">
                    {exp().reading.act === "modify" || exp().reading.act === "operate"
                      ? "Effectful mutation: file edits & command executions."
                      : exp().reading.act === "verify"
                      ? "Testing & verification: test suites & checks."
                      : exp().reading.act === "analyze" || exp().reading.act === "locate"
                      ? "Read-only inspection: search & symbol tracing."
                      : "Direct conversational prose response."}
                  </span>
                  <div class="axis-footer-chip">
                    <span class="mono dim">subsystem: tool loading</span>
                  </div>
                </div>

                {/* 2. HORIZON */}
                <div class="axis-card horizon">
                  <div class="axis-header">
                    <span class="axis-name">2. Horizon</span>
                    <span class="axis-level mono">L{horizonRank(exp().reading.horizon)}/4</span>
                  </div>
                  <div class="axis-val-row">
                    <span class="axis-val">{exp().reading.horizon}</span>
                    <span
                      class="chip"
                      classList={{
                        "chip-tone-success": horizonRank(exp().reading.horizon) >= 4,
                        "chip-phrase": horizonRank(exp().reading.horizon) < 4,
                      }}
                    >
                      {horizonRank(exp().reading.horizon) >= 4
                        ? "durable"
                        : horizonRank(exp().reading.horizon) === 3
                        ? "multi-step"
                        : "ephemeral"}
                    </span>
                  </div>
                  <div class="axis-pip-meter">
                    <For each={[1, 2, 3, 4]}>
                      {(level) => (
                        <span
                          class="pip"
                          classList={{ active: level <= horizonRank(exp().reading.horizon) }}
                        />
                      )}
                    </For>
                  </div>
                  <span class="axis-desc">
                    {horizonRank(exp().reading.horizon) >= 4
                      ? "Opens a durable commitment; outlives the session."
                      : horizonRank(exp().reading.horizon) === 3
                      ? "Runs under a plan in this session; opens no commitment."
                      : "Resolves within the current turn; zero ledger debt."}
                  </span>
                </div>

                {/* 3. STAKES */}
                <div class="axis-card stakes">
                  <div class="axis-header">
                    <span class="axis-name">3. Stakes</span>
                    <span class="axis-level mono">L{stakesRank(exp().reading.stakes)}/4</span>
                  </div>
                  <div class="axis-val-row">
                    <span class="axis-val">{exp().reading.stakes}</span>
                    <span
                      class="chip"
                      classList={{
                        "chip-tone-alert": exp().reading.stakes === "irreversible",
                        "chip-tone-warning": exp().reading.stakes === "costly",
                        "chip-tone-info": exp().reading.stakes === "reversible",
                        "chip-phrase": exp().reading.stakes === "inert",
                      }}
                    >
                      {exp().reading.stakes}
                    </span>
                  </div>
                  <div class="axis-pip-meter">
                    <For each={[1, 2, 3, 4]}>
                      {(level) => (
                        <span
                          class="pip"
                          classList={{
                            active: level <= stakesRank(exp().reading.stakes),
                            "pip-danger": exp().reading.stakes === "irreversible" && level === 4,
                            "pip-warn": exp().reading.stakes === "costly" && level === 3,
                          }}
                        />
                      )}
                    </For>
                  </div>
                  <span class="axis-desc">
                    {exp().reading.stakes === "irreversible"
                      ? "Requires mandatory human sign-off; never bypassed."
                      : exp().reading.stakes === "costly"
                      ? "Metered cloud budget & approval checkpoint."
                      : exp().reading.stakes === "reversible"
                      ? "Filesystem jail with snapshot rollback diff."
                      : "Zero-risk read operation."}
                  </span>
                </div>

                {/* 4. EVIDENCE */}
                <div class="axis-card evidence">
                  <div class="axis-header">
                    <span class="axis-name">4. Evidence</span>
                    <span class="axis-level mono">L{evidenceRank(exp().reading.evidence)}/3</span>
                  </div>
                  <div class="axis-val-row">
                    <span class="axis-val">{exp().reading.evidence}</span>
                  </div>
                  <div class="axis-pip-meter">
                    <For each={[1, 2, 3]}>
                      {(level) => (
                        <span
                          class="pip"
                          classList={{ active: level <= evidenceRank(exp().reading.evidence) }}
                        />
                      )}
                    </For>
                  </div>
                  <span class="axis-desc">
                    {exp().reading.evidence === "audited"
                      ? "Third-party cryptographic affirmation required."
                      : exp().reading.evidence === "verified"
                      ? "Host runtime check (test pass, exit code 0)."
                      : exp().reading.evidence === "cited"
                      ? "Model output accompanied by verified sources."
                      : "Unverified model assertion accepted."}
                  </span>
                </div>

                {/* 5. CLARITY */}
                <div class="axis-card clarity">
                  <div class="axis-header">
                    <span class="axis-name">5. Clarity</span>
                    <span class="axis-tag">Categorical</span>
                  </div>
                  <div class="axis-val-row">
                    <span class="axis-val">{exp().reading.clarity}</span>
                  </div>
                  <span class="axis-desc">
                    {exp().reading.clarity === "clear"
                      ? "Instructions explicit; proceeds without asking."
                      : exp().reading.clarity === "ambiguous"
                      ? exp().reading.stakes === "irreversible"
                        ? "Ambiguity on high stakes: blocks & asks user."
                        : "Ambiguity on low stakes: states assumption & runs."
                      : "Underspecified: falls back to cautious default."}
                  </span>
                  <div class="axis-footer-chip">
                    <span class="mono dim">strategy: {exp().engagement.posture.clarify}</span>
                  </div>
                </div>

                {/* 6. MODALITY */}
                <div class="axis-card modality">
                  <div class="axis-header">
                    <span class="axis-name">6. Modality</span>
                    <span class="axis-tag">Filter</span>
                  </div>
                  <div class="axis-val-row">
                    <span class="axis-val mono" style={{ "font-size": "12px" }}>
                      {exp().engagement.limits.required_modalities.join(", ") || "text"}
                    </span>
                  </div>
                  <span class="axis-desc">
                    Drives frozen ladder routing and model selection filter.
                  </span>
                  <div class="axis-footer-chip">
                    <span class="mono dim">route requirement</span>
                  </div>
                </div>

                {/* 7. ATTENDANCE */}
                <div class="axis-card attendance">
                  <div class="axis-header">
                    <span class="axis-name">7. Attendance</span>
                    <span class="axis-tag">Categorical</span>
                  </div>
                  <div class="axis-val-row">
                    <span class="axis-val">{exp().reading.attendance}</span>
                  </div>
                  <span class="axis-desc">
                    {exp().reading.attendance === "unattended"
                      ? "Unattended daemon: fails closed or defers to inbox."
                      : exp().reading.attendance === "supervised"
                      ? "Supervised bridge: notified on completion/milestones."
                      : "Interactive user present: synchronous stream."}
                  </span>
                  <div class="axis-footer-chip">
                    <span class="mono dim">hil: {exp().engagement.posture.hil}</span>
                  </div>
                </div>
              </div>
            </div>

            {/* Commitment Engine Promotion Decision Callout */}
            <div
              class="commitment-promotion-banner"
              classList={{
                "banner-promoted": exp().engagement.posture.open_commitment,
                "banner-ephemeral": !exp().engagement.posture.open_commitment,
              }}
            >
              <div class="banner-icon-wrap">
                <Show
                  when={exp().engagement.posture.open_commitment}
                  fallback={
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="banner-icon">
                      <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
                    </svg>
                  }
                >
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="banner-icon">
                    <circle cx="12" cy="12" r="10" />
                    <polyline points="12 6 12 12 14 14" />
                  </svg>
                </Show>
              </div>
              <div class="banner-body">
                <div class="banner-title">
                  <strong>
                    {exp().engagement.posture.open_commitment
                      ? "Commitment Engine: Promotion to Durable Obligation"
                      : "Commitment Engine: Ephemeral Interactive Turn"}
                  </strong>
                  <span
                    class="chip"
                    classList={{
                      "chip-tone-success": exp().engagement.posture.open_commitment,
                      "chip-tone-info": !exp().engagement.posture.open_commitment,
                    }}
                  >
                    {exp().engagement.posture.open_commitment ? "durable commitment" : "direct dispatch"}
                  </span>
                </div>
                <div class="banner-desc">
                  <Show
                    when={exp().engagement.posture.open_commitment}
                    fallback={
                      <>
                        This request is classified as single-turn work (horizon: <strong>{exp().reading.horizon}</strong>).
                        The agent executes it directly in-memory. Zero commitment debt is created, keeping your portfolio
                        ledger clean and avoiding unnecessary scheduled polling or inbox reminders.
                      </>
                    }
                  >
                    <>
                      Because horizon is <strong>{exp().reading.horizon}</strong> (≥ session), Vakyartha promotes this work
                      into a durable, tracked <strong>Commitment</strong> with independent audit lineage. Work cannot
                      close as fulfilled until satisfaction criteria achieve at least{" "}
                      <span class="chip chip-phrase">{exp().engagement.limits.min_satisfaction}</span> evidence.
                    </>
                  </Show>
                </div>
              </div>
            </div>

            {/* Derived Safety Envelope & Capability Slicing Grid */}
            <div class="intent-engagement-grid">
              {/* Left Column: Safety Posture & Envelope */}
              <div class="panel">
                <div class="panel-title-row">
                  <div>
                    <h3>Derived Safety Posture &amp; Limits</h3>
                    <p class="dim">Calculated boundary contract restricting what the model is allowed to do.</p>
                  </div>
                </div>
                <div class="posture-specs-table">
                  <div class="spec-row">
                    <span class="spec-label">Managed Execution:</span>
                    <span class="spec-val">
                      <strong>{exp().engagement.posture.managed ? "Yes (Durable Session)" : "No (Direct Interactive)"}</strong>
                    </span>
                  </div>
                  <div class="spec-row">
                    <span class="spec-label">Approval Ceiling:</span>
                    <span class="spec-val">
                      <span
                        class="chip"
                        classList={{
                          "chip-tone-danger": exp().engagement.limits.approval_ceiling === "ask",
                          "chip-tone-warning": exp().engagement.limits.approval_ceiling === "approve-safe",
                          "chip-tone-success": exp().engagement.limits.approval_ceiling === "auto-approve",
                        }}
                      >
                        {exp().engagement.limits.approval_ceiling}
                      </span>
                    </span>
                  </div>
                  <div class="spec-row">
                    <span class="spec-label">Permission Ceiling:</span>
                    <span class="spec-val">
                      <span class="chip chip-phrase">{exp().engagement.limits.permission_ceiling}</span>
                    </span>
                  </div>
                  <div class="spec-row">
                    <span class="spec-label">Pre-Effect Checkpoint:</span>
                    <span class="spec-val mono">
                      {exp().engagement.posture.checkpoint_before_effect ? "Enforced Snapshot (disk)" : "None"}
                    </span>
                  </div>
                  <div class="spec-row">
                    <span class="spec-label">Human-in-the-Loop Mode:</span>
                    <span class="spec-val">
                      <code>{exp().engagement.posture.hil}</code>
                    </span>
                  </div>
                  <div class="spec-row">
                    <span class="spec-label">Stop Profile:</span>
                    <span class="spec-val mono">{exp().engagement.posture.stop}</span>
                  </div>
                  <div class="spec-row">
                    <span class="spec-label">Context Window Slicing:</span>
                    <span class="spec-val mono">{exp().engagement.posture.context}</span>
                  </div>
                  <div class="spec-row">
                    <span class="spec-label">Answer Shape:</span>
                    <span class="spec-val mono">{exp().engagement.posture.delivery.shape}</span>
                  </div>
                </div>
              </div>

              {/* Right Column: Progressive Capability Slicing */}
              <div class="panel">
                <div class="panel-title-row">
                  <div>
                    <h3>Progressive Capability Slicing</h3>
                    <p class="dim">Active narrowing applied to tool advertising (Invariant 10 &amp; 14).</p>
                  </div>
                </div>

                <Show
                  when={exp().narrows.length > 0}
                  fallback={<p class="dim small" style={{ "margin-top": "6px" }}>No capability narrowing restrictions applied beyond default authority.</p>}
                >
                  <div class="narrowing-list">
                    <For each={exp().narrows}>
                      {(narrow) => (
                        <div class="narrowing-item">
                          <span class="narrowing-bullet">◈</span>
                          <span>{narrow}</span>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>

                {/* Model Visible Framing */}
                <Show when={exp().model_visible}>
                  <div class="model-visible-section">
                    <div class="model-visible-header">
                      <span class="eyebrow">Model-Visible Intent Framing (Prompt Injected)</span>
                      <button
                        type="button"
                        class="button ghost small"
                        onClick={() => copyFraming(exp().model_visible!)}
                      >
                        {copiedFraming() ? "Copied ✓" : "Copy Framing"}
                      </button>
                    </div>
                    <pre class="model-visible-code mono">{exp().model_visible}</pre>
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

  const [data, { refetch }] = createResource(selectedAgentId, async (scope) => {
    if (scope !== "all" && scope !== "global") return api.commitments(true, scope);
    const lists = await mapAdminAgents(async (agent) => api.commitments(true, agent.id));
    return {
      commitments: lists.flatMap(({ agent, value }) => value.commitments.map((item) => ({ ...item, admin_agent_id: agent.id }))),
      priorities: lists.flatMap(({ value }) => value.priorities),
    };
  });

  const priorityFor = (item: Commitment) =>
    data()?.priorities.find((p) => p.commitment_id === item.commitment_id);

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
              {(data()?.commitments.length ?? 0) > 0 ? `${metrics().metRatio}%` : "—"}
            </strong>
            <span
              class="chip"
              classList={{
                "chip-tone-success": (data()?.commitments.length ?? 0) > 0,
                "chip-phrase": (data()?.commitments.length ?? 0) === 0,
              }}
            >
              {(data()?.commitments.length ?? 0) > 0 ? "standards met" : "idle (0 tracked)"}
            </span>
          </div>
          <span class="stat-card-hint">
            {(data()?.commitments.length ?? 0) > 0 ? "Lattice verified" : "Lattice active on durable work"}
          </span>
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
              <div class="teach-header">
                <strong>No commitments currently tracked in portfolio</strong>
                <p>
                  Vakyartha opens a durable commitment when a request reads as lasting beyond the current session —
                  such as recurring crons, complex migrations, or work with an explicit done-condition to verify later.
                  Short interactive tasks resolve ephemerally without creating persistent ledger debt.
                </p>
              </div>

              {/* Satisfaction Lattice Standards Guide */}
              <div class="lattice-guide-card">
                <div class="lattice-guide-title">
                  <span>Satisfaction Lattice: The 4 Truth Standards (docs/design/47)</span>
                  <span class="chip chip-phrase">Invariant 3: Verifiable Proof</span>
                </div>
                <div class="lattice-levels-grid">
                  <div class="lattice-level-item l-asserted">
                    <div class="level-badge-row">
                      <span class="level-num">Level 1</span>
                      <span class="level-name">Asserted</span>
                    </div>
                    <p class="level-desc">“The model said so.”</p>
                    <span class="level-sub">Unverified semantic judgement. Minimal standard for non-critical chat.</span>
                  </div>
                  <div class="lattice-level-item l-cited">
                    <div class="level-badge-row">
                      <span class="level-num">Level 2</span>
                      <span class="level-name">Cited</span>
                    </div>
                    <p class="level-desc">“With verified sources.”</p>
                    <span class="level-sub">Model output accompanied by verified quotations, URLs, or file snippets.</span>
                  </div>
                  <div class="lattice-level-item l-observed">
                    <div class="level-badge-row">
                      <span class="level-num">Level 3</span>
                      <span class="level-name">Observed</span>
                    </div>
                    <p class="level-desc">“Runtime verified reality.”</p>
                    <span class="level-sub">Checked against host system: file exists, tests pass, cargo check succeeds.</span>
                  </div>
                  <div class="lattice-level-item l-attested">
                    <div class="level-badge-row">
                      <span class="level-num">Level 4</span>
                      <span class="level-name">Attested</span>
                    </div>
                    <p class="level-desc">“Independent affirmation.”</p>
                    <span class="level-sub">Signed external receipt, human approval token, or third-party oracle proof.</span>
                  </div>
                </div>

                <div class="lifecycle-flow-card">
                  <span class="eyebrow">Commitment Lifecycle State Machine</span>
                  <div class="lifecycle-steps">
                    <span class="step-badge">Proposed</span>
                    <span class="step-arrow">→</span>
                    <span class="step-badge step-active">Active</span>
                    <span class="step-arrow">⇄</span>
                    <span class="step-badge step-waiting">Suspended / Blocked</span>
                    <span class="step-arrow">→</span>
                    <span class="step-badge step-satisfying">Satisfying</span>
                    <span class="step-arrow">→</span>
                    <span class="step-badge step-closed">Closed &#123;fulfilled, partial, failed&#125;</span>
                  </div>
                </div>

                <div class="teach-actions" style={{ "margin-top": "16px" }}>
                  <button
                    type="button"
                    class="button small"
                    onClick={() => switchTab("intent")}
                  >
                    Test Intent Classification in 7-Axis Simulator →
                  </button>
                </div>
              </div>
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
                      priority={priorityFor(commitment)}
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
