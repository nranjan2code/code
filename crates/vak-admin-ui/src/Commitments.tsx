/// The commitment portfolio (docs/design/47-commitment-kernel.md).
///
/// Four questions, in the order an operator actually asks them: what does this
/// agent owe, what will it do next and why, what is stuck and on what, and can
/// I believe the ones it says are finished.
///
/// The last question is the one no other agent surface answers, so it gets the
/// most design. `EvidenceMeter` renders the satisfaction lattice — asserted <
/// cited < observed < attested — with the *required* level marked against the
/// *achieved* one. A commitment closed `fulfilled` on the model's own say-so
/// and one closed on a check the runtime ran are completely different facts,
/// and until you draw them differently they look identical in a status column.
///
/// Rows, not cards. This is a ledger: the operator scans a column, compares
/// across rows, and needs density. Cards would let four commitments fill a
/// screen that should hold thirty.

import { For, Match, Show, Switch, createResource, createSignal } from "solid-js";

import { api } from "./api";
import { PageHeader } from "./display";
import { pushToast } from "./store";
import { timeAgo } from "./time";
import type {
  Commitment,
  CommitmentPriority,
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
///
/// Never colour alone (DESIGN.md): the fill difference is a second channel,
/// and the whole thing carries an `aria-label` stating both levels in words.
function EvidenceMeter(props: {
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
      // A refused closure is the closure invariant speaking, and its message
      // names the evidence that was missing. Surfacing it verbatim is the
      // whole point: the operator learns the rule by hitting it.
      pushToast("alert", String(error instanceof Error ? error.message : error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Show
      when={open()}
      fallback={
        <button class="ghost small" onClick={() => setOpen(true)}>
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
          placeholder="Why"
          value={note()}
          onInput={(e) => setNote(e.currentTarget.value)}
          aria-label="Closing note"
        />
        <button disabled={busy()} onClick={() => void close()}>
          {busy() ? "…" : "Confirm"}
        </button>
        <button class="ghost small" onClick={() => setOpen(false)}>
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
            {props.commitment.spec.reading.act} · {props.commitment.spec.reading.horizon} ·{" "}
            {props.commitment.spec.reading.stakes}
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
                <Show when={props.commitment.blocker}>
                  <div>
                    <span class="k">blocked</span>
                    <span class="v">{props.commitment.blocker}</span>
                  </div>
                </Show>
                <Show when={props.commitment.drift.length > 0}>
                  <div>
                    <span class="k">drift</span>
                    <span class="v">{props.commitment.drift.join("; ")}</span>
                  </div>
                </Show>
              </div>

              <Show when={props.commitment.criteria.length > 0}>
                <ul class="cdetail-criteria">
                  <For each={props.commitment.criteria}>
                    {(criterion) => (
                      <li>
                        <span
                          class="crit-pip"
                          classList={{
                            "crit-pass": criterion.result?.kind === "passed",
                            "crit-fail": criterion.result?.kind === "failed",
                            "crit-unknown": criterion.result?.kind === "unknown",
                          }}
                        />
                        <span class="cdetail-stmt">{criterion.statement}</span>
                        <Show when={criterion.strength}>
                          <span class="cdetail-strength">{criterion.strength}</span>
                        </Show>
                      </li>
                    )}
                  </For>
                </ul>
              </Show>

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
                    closed <b>{closure().verdict}</b> on {closure().strength} evidence
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

export function Commitments() {
  const [showAll, setShowAll] = createSignal(false);
  const [data, { refetch }] = createResource(showAll, (all) => api.commitments(all));

  const priorityFor = (id: string) =>
    data()?.priorities.find((p) => p.commitment_id === id);

  return (
    <section>
      <PageHeader
        title="Commitments"
        description="What this agent owes, what it will work next, and what evidence closed the rest."
      />

      <div class="toolbar">
        <label class="chk">
          <input
            type="checkbox"
            checked={showAll()}
            onChange={(e) => setShowAll(e.currentTarget.checked)}
          />
          Include closed
        </label>
        <button class="ghost small" onClick={() => void refetch()}>
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
          {/* Teaches the mechanism rather than saying "nothing here": the
              reason the list is empty is itself the useful fact. */}
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
        <Match when={data()}>
          <table class="ctable">
            <thead>
              <tr>
                <th class="sr-only">State</th>
                <th>Objective</th>
                <th>Evidence</th>
                <th>Criteria</th>
                <th class="num">Spend</th>
                <th>Priority</th>
                <th>Updated</th>
              </tr>
            </thead>
            <tbody>
              <For each={data()!.commitments}>
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
    </section>
  );
}

export { EvidenceMeter };
