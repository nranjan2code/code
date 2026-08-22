import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import { activeId, isRunning } from "../store";
import { sendPrompt } from "../App";
import * as api from "../api";
import type { PrStatus } from "../types";

interface WatchPrefs {
  watch: boolean;
  auto_fix: boolean;
  auto_merge: boolean;
}

function prefsFor(sid: string): WatchPrefs {
  try {
    const raw = localStorage.getItem(`vak.pr.${sid}`);
    if (raw) return JSON.parse(raw) as WatchPrefs;
  } catch {
    /* fresh */
  }
  return { watch: true, auto_fix: false, auto_merge: false };
}

const CHECK_ICON: Record<string, string> = {
  SUCCESS: "✓",
  FAILURE: "✗",
  CANCELLED: "−",
  TIMED_OUT: "⏱",
};

export default function PrPanel(props: { sessionId: string | null }) {
  const sid = () => props.sessionId ?? activeId();
  const [data, setData] = createSignal<PrStatus | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [prefs, setPrefs] = createSignal<WatchPrefs>({ watch: false, auto_fix: false, auto_merge: false });
  const mergeAttemptedFor = new Set<string>();

  const setPref = (patch: Partial<WatchPrefs>) => {
    const next = { ...prefs(), ...patch };
    setPrefs(next);
    const id = sid();
    if (id) localStorage.setItem(`vak.pr.${id}`, JSON.stringify(next));
  };

  // Load prefs per session.
  createEffect(() => {
    const id = sid();
    if (id) setPrefs(prefsFor(id));
  });

  async function tick() {
    const id = sid();
    if (!id) return;
    setLoading(true);
    setError(null);
    let status: PrStatus | null = null;
    try {
      status = await api.getPr(id);
      setData(status);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }

    if (!status?.pr || !prefs().watch) return;
    const sum = status.summary ?? { pass: 0, fail: 0, pending: 0 };
    const key = `${id}:${status.pr.number}`;

    // Auto-fix: failures + idle session → dispatch a fix run.
    if (prefs().auto_fix && sum.fail > 0 && !isRunning(id)) {
      const failing = (status.checks ?? [])
        .filter((c) => c.conclusion === "FAILURE" || c.conclusion === "TIMED_OUT")
        .map((c) => c.name)
        .join(", ");
      void sendPrompt(
        `PR #${status.pr.number} CI is failing (${failing}). ` +
          `Investigate with \`gh pr checks\` and \`gh run view --log-failed\`, ` +
          `fix the code, commit, and push.`,
      );
      return; // one action per tick
    }

    // Auto-merge: everything green → enable gh auto-merge once.
    if (
      prefs().auto_merge &&
      !mergeAttemptedFor.has(key) &&
      sum.fail === 0 &&
      sum.pending === 0 &&
      sum.pass > 0 &&
      status.pr.state === "OPEN"
    ) {
      mergeAttemptedFor.add(key);
      await api.mergePr(id, status.pr.number).catch((e) => setErr(e));
    }
  }
  function setErr(e: unknown) {
    setError(e instanceof Error ? e.message : String(e));
  }

  // Poll loop only while watching + a session exists.
  createEffect(() => {
    const id = sid();
    void id;
    if (!prefs().watch || !id) return;
    void tick();
    const t = setInterval(() => void tick(), 30_000);
    onCleanup(() => clearInterval(t));
  });

  return (
    <div class="prpane">
      <div class="dock-head">
        <span>Pull request</span>
        <button class="chip sm" onClick={() => void tick()} disabled={loading()}>
          {loading() ? "…" : "refresh"}
        </button>
        <label class="pr-toggle" title="poll CI every 30s">
          <input
            type="checkbox"
            checked={prefs().watch}
            onChange={(e) => setPref({ watch: e.currentTarget.checked })}
          />
          watch
        </label>
        <label class="pr-toggle" title="dispatch a fix run when checks fail">
          <input
            type="checkbox"
            checked={prefs().auto_fix}
            onChange={(e) => setPref({ auto_fix: e.currentTarget.checked })}
          />
          auto-fix
        </label>
        <label class="pr-toggle" title="gh auto-merge once all checks pass">
          <input
            type="checkbox"
            checked={prefs().auto_merge}
            onChange={(e) => setPref({ auto_merge: e.currentTarget.checked })}
          />
          auto-merge
        </label>
      </div>

      <Show when={!error()} fallback={<div class="dock-empty">{error()}</div>}>
        <Show when={data()} fallback={<div class="dock-empty">{sid() ? "loading…" : "select a session"}</div>}>
          {(d) => (
            <div class="pr-body">
              <Show
                when={d().pr}
                fallback={
                  <div class="dock-empty">
                    {d().reason === "no_pr"
                      ? `No PR open for ${d().branch}.`
                      : d().reason === "gh_unavailable"
                        ? "gh CLI unavailable or not authed."
                        : d().error}
                  </div>
                }
              >
                {(pr) => (
                  <>
                    <div class="pr-head">
                      <span class="pr-num">#{pr().number}</span>
                      <span class="pr-title">{pr().title}</span>
                      <span class={`badge pr-${(pr().state || "").toLowerCase()}`}>{pr().state}</span>
                    </div>
                    <Show when={d().summary}>
                      {(s) => (
                        <div class="pr-summary">
                          <span class="adds">✓ {s().pass} passing</span>
                          <Show when={s().fail}>
                            <span class="dels">✗ {s().fail} failing</span>
                          </Show>
                          <Show when={s().pending}>
                            <span class="badge">◌ {s().pending} pending</span>
                          </Show>
                          <button
                            class="btn primary sm"
                            disabled={s().fail > 0 || s().pending > 0 || pr().state !== "OPEN"}
                            title="enable gh auto-merge (squash)"
                            onClick={() => {
                              const id = sid();
                              if (!id) return;
                              void api.mergePr(id, pr().number).then(
                                () => setErr(null),
                                (e) => setErr(e),
                              );
                            }}
                          >
                            merge
                          </button>
                        </div>
                      )}
                    </Show>
                    <div class="pr-checks">
                      <For each={d().checks ?? []}>
                        {(c) => (
                          <div class={`pr-check c-${(c.conclusion ?? c.status ?? "?").toLowerCase()}`}>
                            <span class="ic-check">{CHECK_ICON[c.conclusion ?? ""] ?? "◌"}</span>
                            <span class="ic-name">{c.name}</span>
                            <span class="ic-state">{c.conclusion ?? c.status}</span>
                          </div>
                        )}
                      </For>
                    </div>
                  </>
                )}
              </Show>
            </div>
          )}
        </Show>
      </Show>
    </div>
  );
}
