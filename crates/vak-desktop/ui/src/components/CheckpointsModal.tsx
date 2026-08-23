import { createEffect, createSignal, For, Show } from "solid-js";
import { activeId, historyOpen, isRunning, setHistoryOpen, setNotice } from "../store";
import * as api from "../api";
import type { CheckpointInfo } from "../types";

function timeLabel(iso: string): string {
  const date = new Date(iso);
  const seconds = (Date.now() - date.getTime()) / 1000;
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export default function CheckpointsModal() {
  const [checkpoints, setCheckpoints] = createSignal<CheckpointInfo[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [busySeq, setBusySeq] = createSignal<number | null>(null);
  const [confirming, setConfirming] = createSignal<number | null>(null);

  const id = () => activeId();
  const running = () => isRunning(id());

  createEffect(() => {
    if (!historyOpen() || !id()) return;
    void refresh();
  });

  const refresh = async () => {
    try {
      setCheckpoints((await api.listCheckpoints(id()!)).checkpoints);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const restore = async (seq: number) => {
    setBusySeq(seq);
    try {
      const res = await api.restoreCheckpoint(id()!, seq);
      setNotice({ kind: "info", text: `Rewound to checkpoint ${seq} — restored ${res.restored}, removed ${res.deleted}.` });
      setConfirming(null);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusySeq(null);
    }
  };

  return (
    <Show when={historyOpen()}>
      <div class="modal-back" onClick={() => setHistoryOpen(false)}>
        <div class="modal checkpoints-modal" role="dialog" aria-modal="true" aria-labelledby="history-title" onClick={(e) => e.stopPropagation()}>
          <h3 id="history-title">Time travel — workspace snapshots</h3>
          <p class="history-sub">
            Every turn starts with a full snapshot of the workspace. Restoring rewrites files to
            that moment; the conversation ledger stays intact.
          </p>
          <Show when={running()}>
            <div class="gate-err">A run is active — stop it before rewinding the workspace.</div>
          </Show>
          <Show when={error()}>
            <div class="gate-err">{error()}</div>
          </Show>

          <For each={checkpoints()} fallback={<div class="dock-empty">No snapshots yet. They are captured automatically as turns start.</div>}>
            {(cp) => (
              <div class="checkpoint-row">
                <span class="checkpoint-seq">#{cp.seq}</span>
                <div class="checkpoint-main">
                  <div class="checkpoint-label" title={cp.label}>{cp.label}</div>
                  <div class="checkpoint-meta">
                    {timeLabel(cp.created_at)} · {cp.files} file{cp.files === 1 ? "" : "s"}
                  </div>
                </div>
                <Show
                  when={confirming() === cp.seq}
                  fallback={
                    <button
                      class="chip sm"
                      disabled={running() || busySeq() !== null}
                      title={running() ? "stop the run first" : "restore this snapshot"}
                      onClick={() => setConfirming(cp.seq)}
                    >
                      restore
                    </button>
                  }
                >
                  <span class="checkpoint-confirm">
                    <button class="chip sm danger-chip" disabled={busySeq() !== null} onClick={() => void restore(cp.seq)}>
                      {busySeq() === cp.seq ? "…" : "confirm"}
                    </button>
                    <button class="chip sm" onClick={() => setConfirming(null)}>keep</button>
                  </span>
                </Show>
              </div>
            )}
          </For>

          <div class="bo-foot" style="margin-top:10px">
            <span class="hint">newest work wins · .env and keys are never snapshotted</span>
            <button class="btn primary" onClick={() => setHistoryOpen(false)}>Close</button>
          </div>
        </div>
      </div>
    </Show>
  );
}
