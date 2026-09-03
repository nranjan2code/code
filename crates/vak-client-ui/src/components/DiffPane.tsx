import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import { activeId, isRunning, openInEditor } from "../store";
import { sendPrompt } from "../App";
import * as api from "../api";
import type { DiffResponse } from "../types";
import { parseDiff, parseStatus, type DiffFile } from "../diff";

interface CommentTarget {
  path: string;
  line: number;
}

type Entry =
  | { path: string; badge: string; file: DiffFile }
  | { path: string; badge: string; file?: undefined };

export default function DiffPane(props: { sessionId: string | null }) {
  const [data, setData] = createSignal<DiffResponse | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [comment, setComment] = createSignal<CommentTarget | null>(null);
  const [commentText, setCommentText] = createSignal("");
  const [sentCount, setSentCount] = createSignal(0);

  // The server answers a non-git workspace with { error } and no diff
  // fields, so every read here has to tolerate their absence.
  const files = createMemo<DiffFile[]>(() =>
    parseDiff(`${data()?.diff ?? ""}${data()?.staged_diff ?? ""}`),
  );
  const status = createMemo(() =>
    parseStatus(data()?.status ?? ""),
  );

  const refresh = async () => {
    const id = props.sessionId ?? activeId();
    if (!id) {
      setError("no active session");
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const res = await api.readDiff(id);
      if (res.error) {
        setData(null);
        setError(res.error);
      } else {
        setData(res);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  };

  // Auto-refresh when a run on the active session finishes.
  createEffect((prev: boolean | null) => {
    const running = isRunning(props.sessionId ?? activeId());
    if (prev === true && !running) void refresh();
    return running;
  }, null);
  // Initial fetch once a session exists.
  createEffect(() => {
    if (props.sessionId ?? activeId()) void refresh();
  });

  const submitComment = async () => {
    const c = comment();
    const id = props.sessionId ?? activeId();
    if (!c || !id || !commentText().trim()) return;
    await api.steer(id, `[diff comment ${c.path}:${c.line}] ${commentText().trim()}`);
    setSentCount((n) => n + 1);
    setComment(null);
    setCommentText("");
  };

  // One-shot high-signal review of the working tree (design doc D3).
  const reviewChanges = () => {
    const id = props.sessionId ?? activeId();
    if (isRunning(id) || !entries().length) return;
    // Explicitly targets this pane's own bound session — not necessarily
    // the focused one (a best-of-N child viewed via `diffTarget`) — so
    // "review" always reviews the diff actually on screen.
    void sendPrompt(
      "Review the current uncommitted changes. Report only logic and security findings with specific file:line references — no style nitpicks. End with a verdict: safe to keep, or what must change first.",
      undefined,
      undefined,
      id,
    );
  };

  const entries = createMemo<Entry[]>(() => {
    if (!data()) return [];
    const changed = status().changed.map((c) => ({
      path: c.slice(3),
      badge: c.slice(0, 2),
    }));
    const untracked = status().untracked.map((u) => ({
      path: u,
      badge: "??",
    }));
    const patched = files().map((f) => ({ path: f.path, badge: "~", file: f }));
    return [...changed, ...untracked, ...patched];
  });

  return (
    <div class="diffpane">
      <div class="dock-head">
        <span>Changes{sentCount() ? ` · ${sentCount()} steered` : ""}</span>
        <span class="dock-head-actions">
          <button class="chip sm" onClick={() => void refresh()} disabled={loading()}>
            {loading() ? "…" : "refresh"}
          </button>
          <Show when={props.sessionId ?? activeId()}>
            <button
              class="chip sm"
              title="Ask Vak to review these changes (logic + security)"
              disabled={isRunning(props.sessionId ?? activeId()) || !entries().length}
              onClick={reviewChanges}
            >
              review
            </button>
          </Show>
        </span>
        <Show when={isRunning(props.sessionId)}>
          <span class="hint">click a line → comment → steers the run</span>
        </Show>
      </div>
      <Show when={!error()} fallback={<div class="dock-empty">{error()}</div>}>
        <Show
          when={entries().length}
          fallback={<div class="dock-empty">{data() ? "No changes." : "Press refresh."}</div>}
        >
          <For each={entries()}>
            {(entry) => (
              <details class="dfile" open={!!entry.file}>
                <summary onClick={() => !entry.file && openInEditor(entry.path)}>
                  <code>{entry.path}</code>
                  <Show when={entry.file} fallback={<span class="badge">{entry.badge}</span>}>
                    {(f) => (
                      <>
                        <span class="adds">+{f().adds}</span>
                        <span class="dels">−{f().dels}</span>
                      </>
                    )}
                  </Show>
                </summary>
                <Show when={entry.file}>
                  {(file) => (
                    <div class="hunkwrap">
                      <For each={file().lines}>
                        {(l, i) => (
                          <>
                            <Show
                              when={l.type !== "hunk"}
                              fallback={<div class="dl hunk">{l.text}</div>}
                            >
                              <div
                                class={`dl ${l.type}`}
                                title={l.type === "del" ? undefined : `comment ${l.newNo}`}
                                onClick={() =>
                                  l.type !== "del" &&
                                  l.newNo &&
                                  setComment({ path: entry.path, line: l.newNo })
                                }
                              >
                                <span class="ln">{l.newNo ?? ""}</span>
                                <span class="tx">{l.text}</span>
                              </div>
                            </Show>
                            <Show
                              when={
                                comment()?.path === entry.path &&
                                file()
                                  .lines.findIndex((x) => x.newNo === comment()?.line) === i()
                              }
                            >
                              <div class="dcomment">
                                <input
                                  placeholder="comment for the agent…"
                                  value={commentText()}
                                  onInput={(e) => setCommentText(e.currentTarget.value)}
                                  onKeyDown={(e) => e.key === "Enter" && void submitComment()}
                                />
                                <button class="btn primary sm" onClick={() => void submitComment()}>
                                  send
                                </button>
                              </div>
                            </Show>
                          </>
                        )}
                      </For>
                    </div>
                  )}
                </Show>
              </details>
            )}
          </For>
        </Show>
      </Show>
    </div>
  );
}
