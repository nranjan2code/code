import { createMemo, createSignal, For, Show } from "solid-js";
import { activeId, isRunning, sessions, setBestOfOpen, setTasksOpen } from "../store";
import { activate, newSession } from "../App";
import type { SessionSummary } from "../types";

type Filter = "all" | "active" | "idle";

function timeLabel(iso?: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  const diff = (Date.now() - d.getTime()) / 1000;
  if (diff < 60) return "now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export default function Sidebar() {
  const [filter, setFilter] = createSignal<Filter>("all");
  const [query, setQuery] = createSignal("");

  const visible = createMemo(() => {
    let list = sessions();
    if (filter() === "active") list = list.filter((s) => isRunning(s.session_id));
    else if (filter() === "idle") list = list.filter((s) => !isRunning(s.session_id));
    const q = query().toLowerCase();
    if (q) {
      list = list.filter(
        (s) =>
          (s.title ?? "").toLowerCase().includes(q) ||
          s.session_id.toLowerCase().includes(q),
      );
    }
    return list;
  });

  return (
    <aside class="sidebar">
      <div class="sb-head">
        <div class="brand">
          <span class="brand-mark">◆</span> vakcoder
        </div>
        <div style="display:flex; gap:6px">
          <button
            class="chip"
            title="Scheduled tasks — recurring runs in worktrees"
            onClick={() => setTasksOpen(true)}
          >
            ⏱
          </button>
          <button
            class="chip"
            title="Best of N — fan a prompt across isolated worktrees"
            disabled={!activeId()}
            onClick={() => setBestOfOpen(true)}
          >
            N×
          </button>
          <button class="btn primary sb-new" title="New session (⌘N)" onClick={() => void newSession()}>
            + New
          </button>
        </div>
      </div>

      <input
        class="sb-search"
        type="search"
        placeholder="Search sessions…"
        value={query()}
        onInput={(e) => setQuery(e.currentTarget.value)}
      />

      <div class="sb-filters">
        <For each={["all", "active", "idle"] as Filter[]}>
          {(f) => (
            <button
              class="chip"
              classList={{ on: filter() === f }}
              onClick={() => setFilter(f)}
            >
              {f}
            </button>
          )}
        </For>
      </div>

      <div class="sb-list">
        <Show
          when={visible().length}
          fallback={<div class="sb-empty">No sessions yet. Press ⌘N to start.</div>}
        >
          <For each={visible()}>
            {(s: SessionSummary) => (
              <button
                class="sb-item"
                classList={{ active: activeId() === s.session_id }}
                onClick={() => void activate(s.session_id)}
              >
                <span
                  class="dot"
                  classList={{ run: isRunning(s.session_id) }}
                />
                <span class="sb-title">{s.title || s.session_id.slice(0, 8)}</span>
                <span class="sb-time">{timeLabel(s.updated_at)}</span>
              </button>
            )}
          </For>
        </Show>
      </div>
    </aside>
  );
}
