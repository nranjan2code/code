import { trapFocus } from "../focusTrap";
import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import {
  activeId,
  applyEvent,
  bestOfOpen,
  bestOfRuns,
  diffTarget,
  isRunning,
  itemsOf,
  setBestOfOpen,
  setBestOfRuns,
  setDiffTarget,
  setDockTab,
  setNotice,
} from "../store";
import { watchSession } from "../streamHub";
import * as api from "../api";
import { Markdown } from "./ChatPane";

const streams = new Map<string, () => void>();

function RunCard(props: { run: { session_id: string; branch: string } }) {
  const [verdict, setVerdict] = createSignal<"kept" | "discarded" | null>(null);
  const [err, setErr] = createSignal<string | null>(null);
  const cid = () => props.run.session_id;
  const running = () => isRunning(cid()) && !verdict();

  // Live events for this candidate land in their own bucket keys.
  createEffect(() => {
    const id = cid();
    if (streams.has(id)) return;
    const stop = watchSession(id, { agent: (ev) => applyEvent(id, ev, {}) });
    streams.set(id, stop);
    onCleanup(() => {
      stop();
      streams.delete(id);
    });
  });

  const finalText = () => {
    const list = itemsOf(cid());
    for (let i = list.length - 1; i >= 0; i--) {
      const it = list[i];
      if (it.kind === "assistant" && it.text.trim()) return it.text;
    }
    return "";
  };

  const act = async (kind: "keep" | "discard") => {
    setErr(null);
    try {
      if (kind === "keep") await api.keepRun(cid());
      else await api.discardRun(cid());
      setVerdict(kind === "keep" ? "kept" : "discarded");
      if (kind !== "keep" && diffTarget() === cid()) setDiffTarget(null);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div class="bo-card" classList={{ done: !running() }}>
      <div class="bo-head">
        <span class="dot" classList={{ run: running() }} role="img"
              aria-label={running() ? "Running" : "Idle"} />
        <span class="bo-branch">{props.run.branch}</span>
        <Show when={verdict()}>
          <span class={`badge bo-${verdict()}`}>{verdict()}</span>
        </Show>
      </div>
      <div class="bo-body">
        <Show
          when={finalText()}
          fallback={<div class="sysnote">working…</div>}
        >
          {(t) => <Markdown text={t()} />}
        </Show>
      </div>
      <Show when={err()}>
        <div class="bo-err">{err()}</div>
      </Show>
      <Show when={!verdict() && !running()}>
        <div class="bo-actions">
          <button
            class="btn primary"
            onClick={() => {
              setDiffTarget(cid());
              setDockTab("diff");
            }}
          >
            diff
          </button>
          <button class="btn primary" onClick={() => void act("keep")}>keep</button>
          <button class="btn danger" onClick={() => void act("discard")}>discard</button>
        </div>
      </Show>
      <Show when={diffTarget() === cid()}>
        <div class="hint" style="padding: 0 10px 8px">shown in diff pane</div>
      </Show>
    </div>
  );
}

export default function BestOfNDialog() {
  const runs = () => bestOfRuns();
  const close = () => {
    for (const stop of streams.values()) stop();
    streams.clear();
    setBestOfOpen(false);
    setBestOfRuns(null);
  };
  const [prompt, setPrompt] = createSignal("");
  const [n, setN] = createSignal(2);
  const [starting, setStarting] = createSignal(false);
  const start = () => {
    const anchor = activeId();
    if (!anchor || !prompt().trim()) return;
    setStarting(true);
    void api
      .startBestOfN(anchor, prompt().trim(), n())
      .then((res) => setBestOfRuns(res.runs))
      .catch((e) => setNotice({ kind: "error", text: `Comparison failed: ${e instanceof Error ? e.message : String(e)}` }))
      .finally(() => setStarting(false));
  };

  return (
    <div class="modal-back" onClick={close}>
      <div class="modal bo-modal" role="dialog" aria-modal="true" aria-labelledby="compare-title" onClick={(e) => e.stopPropagation()} use:trapFocus>
        <h3 id="compare-title">Compare approaches — isolated worktrees</h3>
        <Show
          when={runs()}
          fallback={
            <div class="bo-config">
              <textarea
                rows={4}
                placeholder="The prompt every candidate will run…"
                value={prompt()}
                onInput={(e) => setPrompt(e.currentTarget.value)}
              />
              <label class="bo-n">
                candidates
                <select value={String(n())} onChange={(e) => setN(Number(e.currentTarget.value))}>
                  <option value="2">2</option>
                  <option value="3">3</option>
                  <option value="4">4</option>
                </select>
              </label>
              <button class="btn primary" disabled={!prompt().trim() || starting()} onClick={start}>
                {starting() ? "starting…" : `Run ${n()}×`}
              </button>
            </div>
          }
        >
          {(rs) => (
            <div class="bo-grid">
              <For each={rs()}>{(r) => <RunCard run={r} />}</For>
            </div>
          )}
        </Show>
        <div class="bo-foot">
          <span class="hint">
            keep merges the branch into your checkout · discard throws it away
          </span>
          <button class="btn primary" onClick={close}>Close</button>
        </div>
      </div>
    </div>
  );
}
