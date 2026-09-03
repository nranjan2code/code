import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import type { JSX } from "solid-js";
import * as api from "../api";
import type { ActiveSubagent } from "../api";
import Icon from "./Icon";

/**
 * Live subagent attach/steer panel (dock tab). Polls the session's
 * registry while visible: children stream their tool calls into the main
 * transcript already — this adds the missing half, steering and stopping
 * a running child without touching the parent run.
 */
export default function SubagentsPanel(props: { sessionId: string | null }): JSX.Element {
  const [children, setChildren] = createSignal<ActiveSubagent[]>([]);
  const [drafts, setDrafts] = createSignal<Record<string, string>>({});
  const [error, setError] = createSignal<string | null>(null);
  let timer: number | undefined;

  const refresh = async () => {
    const id = props.sessionId;
    if (!id) {
      setChildren([]);
      return;
    }
    try {
      const res = await api.listSubagents(id);
      setChildren(res.subagents);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  createEffect(() => {
    void props.sessionId;
    void refresh();
    window.clearInterval(timer);
    timer = window.setInterval(() => void refresh(), 1500);
  });

  onCleanup(() => window.clearInterval(timer));

  const send = async (child: string) => {
    const id = props.sessionId;
    const text = (drafts()[child] ?? "").trim();
    if (!id || !text) return;
    try {
      await api.steerSubagent(id, child, text);
      setDrafts((d) => ({ ...d, [child]: "" }));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const stop = async (child: string) => {
    const id = props.sessionId;
    if (!id) return;
    try {
      await api.stopSubagent(id, child);
      void refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div class="subagents-panel" role="tabpanel" aria-label="Running subagents">
      <Show when={error()}>
        <div class="subagent-error">{error()}</div>
      </Show>
      <Show
        when={props.sessionId}
        fallback={<div class="subagent-empty">Open a task to see its subagents.</div>}
      >
        <Show
          when={children().length > 0}
          fallback={
            <div class="subagent-empty">
              No subagents running. Ask for parallel research or delegation and live children appear here.
            </div>
          }
        >
          <For each={children()}>
            {(child) => (
              <div class="subagent-card">
                <div class="subagent-head">
                  <strong title={child.id}>{child.label}</strong>
                  <span class="subagent-elapsed">{child.elapsed_secs}s</span>
                  <button
                    class="subagent-stop"
                    title="Stop this subagent"
                    aria-label={`Stop ${child.label}`}
                    onClick={() => void stop(child.id)}
                  >
                    <Icon name="stop" size={13} />
                    Stop
                  </button>
                </div>
                <div class="subagent-steer">
                  <input
                    type="text"
                    placeholder="Steer this subagent…"
                    value={drafts()[child.id] ?? ""}
                    onInput={(e) => setDrafts((d) => ({ ...d, [child.id]: e.currentTarget.value }))}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && !e.isComposing) void send(child.id);
                    }}
                  />
                  <button
                    class="settings-button"
                    disabled={!(drafts()[child.id] ?? "").trim()}
                    onClick={() => void send(child.id)}
                  >
                    Steer
                  </button>
                </div>
              </div>
            )}
          </For>
        </Show>
      </Show>
    </div>
  );
}
