import { createEffect, createSignal, For, onCleanup, Show, onMount } from "solid-js";
import type { JSX } from "solid-js";
import * as api from "../api";
import type { ActiveWorker } from "../api";
import Icon from "./Icon";
import AgentMark from "./AgentMark";

/**
 * Live worker attach/steer panel (dock tab). Polls the session's
 * registry while visible: children stream their tool calls into the main
 * transcript already — this adds the missing half, steering and stopping
 * a running child without touching the parent run.
 */
export default function WorkersPanel(props: { sessionId: string | null }): JSX.Element {
  const [children, setChildren] = createSignal<ActiveWorker[]>([]);
  const [drafts, setDrafts] = createSignal<Record<string, string>>({});
  const [error, setError] = createSignal<string | null>(null);
  const [profiles, setProfiles] = createSignal<api.Agent[]>([]);
  let timer: number | undefined;
  onMount(() => { void api.listAgents().then((res) => setProfiles(res.agents)).catch(() => { /* optional enhancement */ }); });
  const profileFor = (label: string) => profiles().find((profile) => profile.name.toLowerCase() === label.toLowerCase());
  const profileForChild = (child: ActiveWorker) => profiles().find((profile) => profile.id === child.agent_id) ?? profileFor(child.label);

  const refresh = async () => {
    const id = props.sessionId;
    if (!id) {
      setChildren([]);
      return;
    }
    try {
      const res = await api.listWorkers(id);
      setChildren(res.workers);
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
      await api.steerWorker(id, child, text);
      setDrafts((d) => ({ ...d, [child]: "" }));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const stop = async (child: string) => {
    const id = props.sessionId;
    if (!id) return;
    try {
      await api.stopWorker(id, child);
      void refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div class="workers-panel" role="tabpanel" aria-label="Running workers">
      <Show when={error()}>
        <div class="worker-error">{error()}</div>
      </Show>
      <Show
        when={props.sessionId}
        fallback={<div class="worker-empty">Open a task to see its workers.</div>}
      >
        <Show
          when={children().length > 0}
          fallback={
            <div class="worker-empty">
              No workers running. Ask for parallel research or delegation and live children appear here.
            </div>
          }
        >
          <For each={children()}>
            {(child) => (
              <div class="worker-card">
                <div class="worker-head">
                  <span class="worker-identity"><AgentMark character={profileForChild(child)?.character} size={22} working /><span><strong title={child.id}>{child.label}</strong><small>{profileForChild(child)?.name ? `with ${profileForChild(child)!.name} · revision ${child.agent_revision ?? profileForChild(child)!.revision}` : "Vak delegated this work"}</small></span></span>
                  <span class="worker-elapsed">{child.elapsed_secs}s</span>
                  <button
                    class="worker-stop"
                    title="Stop this worker"
                    aria-label={`Stop ${child.label}`}
                    onClick={() => void stop(child.id)}
                  >
                    <Icon name="stop" size={13} />
                    Stop
                  </button>
                </div>
                <div class="worker-steer">
                  <input
                    type="text"
                    placeholder="Steer this worker…"
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
