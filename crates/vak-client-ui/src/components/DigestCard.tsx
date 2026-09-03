import { createEffect, createSignal, For, Show } from "solid-js";
import * as api from "../api";
import type { DigestReport } from "../api";

function usd(n: number): string {
  return n >= 100 ? `$${n.toFixed(0)}` : `$${n.toFixed(2)}`;
}

function tokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

/** Weekly usage digest (docs/design/29-personal-os.md P3): ledger + memory + proposals over the trailing window. */
export default function DigestCard() {
  const [days, setDays] = createSignal(7);
  const [report, setReport] = createSignal<DigestReport | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  const refresh = async () => {
    setBusy(true);
    setError(null);
    try {
      setReport(await api.digest(days()));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  createEffect(() => {
    const _ = days();
    void refresh();
  });

  // Top-3 models by spend; unpriced rows never masquerade as cheap models.
  const topModels = (): { name: string; usd: number; rows: number }[] =>
    Object.entries(report()?.by_model ?? {})
      .map(([name, r]) => ({ name, usd: r.usd, rows: r.rows }))
      .sort((a, b) => b.usd - a.usd)
      .slice(0, 3);

  return (
    <section class="operation-card digest-card">
      <header>
        <div>
          <h3>Usage digest</h3>
          <p>Spend, tokens, memory and skill activity</p>
        </div>
        <label class="bo-n">
          <span class="hint">window</span>
          <select value={String(days())} onChange={(e) => setDays(Number(e.currentTarget.value))} aria-label="Digest window in days">
            {[1, 7, 14, 30].map((n) => (
              <option value={String(n)}>{n}d</option>
            ))}
          </select>
        </label>
        <button class="settings-button" disabled={busy()} onClick={() => void refresh()}>{busy() ? "…" : "Refresh"}</button>
      </header>
      <Show when={error()}>
        <div class="settings-warning">Could not load the digest: {error()}</div>
      </Show>
      <Show when={report()}>
        {(r) => (
          <>
            <div class="digest-tiles">
              <div class="digest-tile"><b>Total spend</b><strong>{usd(r().total_usd)}</strong></div>
              <div class="digest-tile"><b>Tokens</b><strong>{tokens(r().input_tokens)} ↑ {tokens(r().output_tokens)} ↓</strong></div>
              <div class="digest-tile"><b>Dispatches</b><strong>{r().dispatches}</strong></div>
              <div class="digest-tile"><b>Sessions</b><strong>{r().distinct_sessions.length}</strong></div>
              <div class="digest-tile"><b>Memory notes</b><strong>{r().memory_notes_appended}</strong></div>
              <div class="digest-tile" classList={{ warn: r().unpriced_rows > 0 }}>
                <b>Unpriced</b><strong>{r().unpriced_rows}</strong>
                <Show when={r().unpriced_rows > 0}><small>excluded from $ total</small></Show>
              </div>
            </div>
            <div class="operation-rollups">
              <For each={topModels()} fallback={<p class="operation-muted">No model activity in this window.</p>}>
                {(m, i) => (
                  <div>
                    <span><em class="digest-rank">#{i() + 1}</em> {m.name}</span>
                    <small>{usd(m.usd)} · {m.rows} dispatch{m.rows === 1 ? "" : "es"}</small>
                  </div>
                )}
              </For>
            </div>
            <p class="settings-hint">skill proposals opened: {r().skill_proposals_opened}{r().since ? ` · since ${new Date(r().since!).toLocaleDateString()}` : ""}</p>
          </>
        )}
      </Show>
    </section>
  );
}
