import { createEffect, createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import * as api from "../api";
import type { Commitment, CriterionState, Satisfaction, Verdict } from "../api";
import { backend } from "../store";
import { relTime } from "../time";
import Icon, { type IconName } from "./Icon";

/// The commitment portfolio, brought into the workspace client.
///
/// This used to exist only in the admin console
/// (crates/vak-admin-ui/src/Commitments.tsx) — a durable obligation the
/// runtime will verify against the world is exactly workspace information,
/// and there was no way to see or close one without leaving to a second
/// application. This is a lean read of the same model, scoped to the
/// active workspace (`spec.cwd`), for the one job that matters here: what
/// does this workspace still owe, and can I believe the ones it says are
/// done.
///
/// The full ledger — every workspace, scheduler-priority breakdown, drift
/// history — stays the admin console's job (invariant 26: an evidence
/// projection, not a second source of truth). This is not a second
/// portfolio; it is the same read model with a narrower lens.

const LATTICE: Satisfaction[] = ["asserted", "cited", "observed", "attested"];
const STRENGTH_BLURB: Record<Satisfaction, string> = {
  asserted: "the model said so",
  cited: "the model said so, with sources",
  observed: "the runtime checked it against the world",
  attested: "an independent party confirmed it",
};
function rank(s: Satisfaction): number {
  return LATTICE.indexOf(s);
}

/** The satisfaction lattice, drawn. See DESIGN.md "Evidence Meter" — solid
 * up to what was achieved, hollow beyond it, a rule marking the required
 * level, and a spoken label so the state never rides on hue alone. */
function EvidenceMeter(props: { achieved: Satisfaction; required: Satisfaction }) {
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
      <span class="ev-label" classList={{ "ev-label-short": short() }}>
        {short() ? `${props.achieved} · needs ${props.required}` : props.achieved}
      </span>
    </div>
  );
}

/** Weakest strength among passed criteria, or `asserted` when none have. */
function achievedStrength(commitment: Commitment): Satisfaction {
  const strengths = commitment.criteria
    .filter((c) => c.result?.kind === "passed")
    .map((c) => c.strength ?? "asserted");
  if (strengths.length === 0) return "asserted";
  return strengths.reduce((weakest, s) => (rank(s) < rank(weakest) ? s : weakest));
}

function CriterionRow(props: { criterion: CriterionState }) {
  const c = props.criterion;
  const icon = (): IconName => (c.result?.kind === "passed" ? "check" : c.result?.kind === "failed" ? "close" : "timer");
  return (
    <div class="commit-criterion" classList={{ passed: c.result?.kind === "passed", failed: c.result?.kind === "failed" }}>
      <Icon name={icon()} size={12} />
      <span class="commit-criterion-text">{c.statement}</span>
      <Show when={!c.required}>
        <span class="chip sm">optional</span>
      </Show>
    </div>
  );
}

const VERDICTS: Verdict[] = ["fulfilled", "partial", "failed", "abandoned"];

function CommitmentRow(props: { commitment: Commitment; onClosed: () => void }) {
  const [open, setOpen] = createSignal(false);
  const [closing, setClosing] = createSignal(false);
  const [note, setNote] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const c = () => props.commitment;
  const outstanding = createMemo(() => c().criteria.filter((it) => it.required && it.result?.kind !== "passed"));
  const canClose = () => c().phase !== "closed";

  const close = async (verdict: Verdict) => {
    setClosing(true);
    setError(null);
    try {
      await api.closeCommitment(c().commitment_id, verdict, note());
      props.onClosed();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setClosing(false);
    }
  };

  return (
    <div class="commit-row" classList={{ closed: c().phase === "closed" }}>
      <button class="commit-row-head" onClick={() => setOpen((v) => !v)} aria-expanded={open()}>
        <span class="commit-obj">{c().spec.objective}</span>
        <span class="commit-phase" classList={{ [`phase-${c().phase}`]: true }}>
          {c().phase === "closed" ? (c().closure?.verdict ?? "closed") : c().phase}
        </span>
        <EvidenceMeter achieved={achievedStrength(c())} required={c().spec.min_satisfaction} />
        <Show when={c().phase !== "closed"}>
          <span class="commit-outstanding">{outstanding().length} open</span>
        </Show>
        <span class="tool-chev"><Icon name="chevron" size={13} /></span>
      </button>
      <Show when={open()}>
        <div class="commit-detail">
          <For each={c().criteria}>{(criterion) => <CriterionRow criterion={criterion} />}</For>
          <Show when={c().blocker}>
            {(blocker) => <div class="commit-blocker"><Icon name="warning" size={12} /> {blocker()}</div>}
          </Show>
          <Show when={c().closure}>
            {(closure) => (
              <div class="commit-closure">
                Closed {closure().verdict} · {relTime(closure().closed_at)}
                <Show when={closure().note}> — {closure().note}</Show>
              </div>
            )}
          </Show>
          <Show when={canClose()}>
            <div class="commit-close-form">
              <input
                placeholder="Note for the ledger (optional)"
                value={note()}
                onInput={(e) => setNote(e.currentTarget.value)}
              />
              <For each={VERDICTS}>
                {(v) => (
                  <button class="chip sm" disabled={closing()} onClick={() => void close(v)}>
                    {v}
                  </button>
                )}
              </For>
            </div>
            <Show when={error()}>
              <div class="commit-error">{error()}</div>
            </Show>
          </Show>
        </div>
      </Show>
    </div>
  );
}

export default function CommitmentsPanel() {
  const [all, setAll] = createSignal(false);
  const [commitments, setCommitments] = createSignal<Commitment[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  let timer: number | undefined;

  const refresh = async () => {
    try {
      const res = await api.listCommitments(all());
      setCommitments(res.commitments);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  createEffect(() => {
    void all();
    void refresh();
    window.clearInterval(timer);
    timer = window.setInterval(() => void refresh(), 5000);
  });
  onCleanup(() => window.clearInterval(timer));

  // Client-side scope to the active workspace: the server's own read
  // spans every workspace it has ever tracked (like sessions do), and
  // `spec.cwd` is the same field a session's own `cwd` mirrors.
  const scoped = createMemo(() =>
    commitments().filter((c) => c.spec.cwd === backend().cwd),
  );
  const open = createMemo(() => scoped().filter((c) => c.phase !== "closed"));
  const closed = createMemo(() => scoped().filter((c) => c.phase === "closed"));

  return (
    <div class="commit-panel" role="tabpanel" aria-label="Commitments">
      <div class="dock-head">
        <span>Commitments{open().length ? ` · ${open().length} open` : ""}</span>
        <span class="dock-head-actions">
          <button class="chip sm" classList={{ on: all() }} onClick={() => setAll((v) => !v)}>
            {all() ? "hide closed" : "show closed"}
          </button>
          <button class="chip sm" onClick={() => void refresh()}>refresh</button>
        </span>
      </div>
      <Show when={error()}>
        <div class="dock-empty">{error()}</div>
      </Show>
      <div class="commit-list">
        <Show
          when={scoped().length}
          fallback={
            <div class="dock-empty">
              No durable commitments for this workspace. One opens when a run's
              reading calls for work that outlives the turn.
            </div>
          }
        >
          <For each={open()}>{(c) => <CommitmentRow commitment={c} onClosed={refresh} />}</For>
          <Show when={all() && closed().length}>
            <div class="commit-section-label">Closed</div>
            <For each={closed()}>{(c) => <CommitmentRow commitment={c} onClosed={refresh} />}</For>
          </Show>
        </Show>
      </div>
    </div>
  );
}
