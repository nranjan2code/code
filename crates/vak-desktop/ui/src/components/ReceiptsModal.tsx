import { trapFocus } from "../focusTrap";
import { createEffect, createSignal, For, Show } from "solid-js";
import { activeId, receiptsOpen, setReceiptsOpen } from "../store";
import * as api from "../api";
import type { DispatchAttempt, WorkReceipt } from "../types";

function purposeLabel(p: WorkReceipt["purpose"]): string {
  switch (p) {
    case "execute":
      return "model step";
    case "summarize":
      return "compaction / handoff";
    case "verify":
      return "completion audit";
    default:
      return p;
  }
}

function settlementClass(s: DispatchAttempt["settlement"]): string {
  switch (s) {
    case "ok":
      return "good";
    case "failed":
      return "bad";
    default:
      return "";
  }
}

function reasonLabel(r: DispatchAttempt["reason"]): string {
  switch (r) {
    case "route_fallback":
      return "route fallback";
    case "endurance_retry":
      return "endurance retry";
    default:
      return r;
  }
}

/** Leg attribution for one attempt: the per-attempt stamp when a fallback
 * walk renamed it, else the receipt-level leg. */
function attemptLeg(r: WorkReceipt, a: DispatchAttempt): string {
  const provider = a.provider ?? r.provider;
  const model = a.model ?? r.model;
  const tail = `${provider}/${model}`;
  if (a.provider || a.model) return `${tail} · stamped at dispatch`;
  return tail;
}

export default function ReceiptsModal() {
  const [receipts, setReceipts] = createSignal<WorkReceipt[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [expanded, setExpanded] = createSignal<number | null>(null);

  createEffect(() => {
    if (!receiptsOpen() || !activeId()) return;
    void refresh();
  });

  const refresh = async () => {
    try {
      setReceipts(await api.receipts(activeId()!));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const totalAttempts = () =>
    receipts().reduce((n, r) => n + r.attempts.length, 0);
  const fallbackWalks = () =>
    receipts().filter((r) =>
      r.attempts.some((a) => a.reason === "route_fallback"),
    ).length;

  return (
    <Show when={receiptsOpen()}>
      <div class="modal-back" onClick={() => setReceiptsOpen(false)}>
        <div
          class="modal checkpoints-modal receipts-modal"
          role="dialog"
          aria-modal="true"
          aria-labelledby="receipts-title"
          onClick={(e) => e.stopPropagation()}
          use:trapFocus
        >
          <h3 id="receipts-title">Dispatch forensics</h3>
          <p class="history-sub">
            Every paid model call in this task, exactly as recorded in the
            append-only ledger. Expand a step to see its frozen-ladder
            attempt walk — retries, route fallbacks, and settlements.
          </p>
          <Show when={error()}>
            <div class="gate-err">{error()}</div>
          </Show>

          <Show
            when={receipts().length}
            fallback={
              <div class="dock-empty">
                No dispatches yet. Receipts appear here as model calls settle.
              </div>
            }
          >
            <div class="receipt-summary">
              {receipts().length} step{receipts().length === 1 ? "" : "s"} ·{" "}
              {totalAttempts()} dispatch{totalAttempts() === 1 ? "" : "es"}
              <Show when={fallbackWalks()}>
                {" "}
                · {fallbackWalks()} ladder walk
                {fallbackWalks() === 1 ? "" : "s"}
              </Show>
            </div>
            <For each={receipts()}>
              {(r, i) => {
                const open = () => expanded() === i();
                const won = (a: DispatchAttempt) =>
                  r.winning_attempt != null &&
                  a.ordinal === r.winning_attempt;
                return (
                  <div class="receipt-row" classList={{ open: open() }}>
                    <button
                      type="button"
                      class="receipt-head"
                      onClick={() => setExpanded(open() ? null : i())}
                      aria-expanded={open()}
                    >
                      <span class="receipt-ord">#{i() + 1}</span>
                      <span class="receipt-main">
                        <span class="receipt-label">
                          {purposeLabel(r.purpose)}
                          <Show when={r.winning_attempt != null}>
                            <span class="settings-status good receipt-won">
                              delivered
                            </span>
                          </Show>
                        </span>
                        <span class="checkpoint-meta">
                          {r.provider}/{r.model} · {r.attempts.length}{" "}
                          attempt{r.attempts.length === 1 ? "" : "s"}
                          {r.winning_attempt != null
                            ? ` · ${r.attempts[r.winning_attempt].latency_ms} ms`
                            : " · never settled"}
                        </span>
                      </span>
                      <span class="tool-chev receipt-chev" classList={{ flip: open() }}>
                        ›
                      </span>
                    </button>
                    <Show when={open()}>
                      <div class="receipt-attempts">
                        <For each={r.attempts}>
                          {(a) => (
                            <div class="attempt-row">
                              <span class={`settings-status ${settlementClass(a.settlement)} attempt-settle`}>
                                {a.settlement}
                              </span>
                              <span class="attempt-body">
                                <span class="attempt-line">
                                  <span class="chip sm">{reasonLabel(a.reason)}</span>
                                  <Show when={won(a)}>
                                    <span class="chip sm good-chip">winning</span>
                                  </Show>
                                  <span class="attempt-domain">{a.domain}</span>
                                  <span class="attempt-meta">{a.latency_ms} ms</span>
                                  <Show when={a.usage}>
                                    <span class="attempt-meta">
                                      ↑{a.usage!.input_tokens ?? 0} ↓
                                      {a.usage!.output_tokens ?? 0}
                                    </span>
                                  </Show>
                                </span>
                                <span class="checkpoint-meta">
                                  {attemptLeg(r, a)}
                                </span>
                                <Show when={a.error}>
                                  <span class="attempt-error" title={a.error!}>
                                    {a.error}
                                  </span>
                                </Show>
                              </span>
                            </div>
                          )}
                        </For>
                      </div>
                    </Show>
                  </div>
                );
              }}
            </For>
          </Show>

          <div class="bo-foot" style="margin-top:10px">
            <span class="hint">append-only ledger · billing without a verdict settles unknown</span>
            <button class="btn primary" onClick={() => void refresh()}>Refresh</button>
            <button class="btn primary" onClick={() => setReceiptsOpen(false)}>Close</button>
          </div>
        </div>
      </div>
    </Show>
  );
}
