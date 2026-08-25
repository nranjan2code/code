import { createSignal } from "solid-js";
import { createStore } from "solid-js/store";
import type {
  AgentEvent,
  AssistantMessage,
  BackendInfo,
  ContentBlock,
  Health,
  Message,
  SessionSummary,
  Usage,
} from "./types";

export type Density = "normal" | "verbose" | "summary";

export type Item =
  | { kind: "user"; text: string }
  | { kind: "assistant"; key: string; text: string; streaming: boolean }
  | { kind: "thinking"; key: string; text: string; done: boolean }
  | {
      kind: "tool";
      id: string;
      name: string;
      argsJson: string;
      done: boolean;
      isError: boolean;
      preview: string | null;
    }
  | {
      kind: "approval";
      id: string;
      tool: string;
      argsJson: string;
      reason: string;
      resolved: null | "allowed" | "denied" | "gone";
    }
  | { kind: "subagent"; label: string; lines: string[]; open: boolean; isError: boolean }
  | { kind: "system"; text: string };

export const [backend, setBackend] = createSignal<BackendInfo>({ ready: false, recent_projects: [] });
export const [workspaceSwitching, setWorkspaceSwitching] = createSignal(false);
export const [sessions, setSessions] = createSignal<SessionSummary[]>([]);
export const [activeId, setActiveId] = createSignal<string | null>(null);
export const [health, setHealth] = createSignal<Health | null>(null);
// Provider/model picker state; setupNeeded keeps the project gate up until a
// usable credential exists for the current provider.
export const [providers, setProviders] = createSignal<import("./types").ProvidersResponse | null>(null);
export const [setupNeeded, setSetupNeeded] = createSignal(false);
export const [density, setDensity] = createSignal<Density>("normal");
export const [dockTab, setDockTab] = createSignal<"preview" | "diff" | "terminal" | "editor" | "pr" | "agents" | null>(null);
export const [showShortcuts, setShowShortcuts] = createSignal(false);
export const [settingsOpen, setSettingsOpen] = createSignal(false);
export const [hydratingId, setHydratingId] = createSignal<string | null>(null);
export const [sidebarOpen, setSidebarOpen] = createSignal(true);
export const [sidebarWidth, setSidebarWidth] = createSignal(252);
export const [dockWidth, setDockWidth] = createSignal(520);

/**
 * Split view: the session shown in the non-focused pane. `activeId` always
 * names the FOCUSED pane's session (composer, approvals, stop, dock all
 * follow it); `splitFocused` says whether that focus currently sits on the
 * right pane. Pane positions never move — focusing a pane swaps contents.
 */
export const [splitId, setSplitId] = createSignal<string | null>(null);
export const [splitFocused, setSplitFocused] = createSignal(false);
const storedSplitRatio = Number(localStorage.getItem("vakcoder.splitRatio"));
export const [splitRatio, setSplitRatio] = createSignal(
  Number.isFinite(storedSplitRatio) && storedSplitRatio >= 0.25 && storedSplitRatio <= 0.75 ? storedSplitRatio : 0.5,
);
/** Session id rendered in a given pane; focus decides which side is active. */
export function paneSessions(): { left: string | null; right: string | null } {
  if (!splitId()) return { left: activeId(), right: null };
  return splitFocused()
    ? { left: splitId(), right: activeId() }
    : { left: activeId(), right: splitId() };
}
export type Notice = { kind: "error" | "info"; text: string };
export const [notice, setNotice] = createSignal<Notice | null>(null);

/**
 * Live retry state per session.
 *
 * RetryScheduled is emitted once, before the backoff sleep, and the attempt
 * itself is silent — so a transcript line saying "retrying in 0.6s" was the
 * last thing a user ever saw. Holding the state lets the header keep saying
 * "retrying" until real progress (a delta, a tool call, an end) arrives.
 */
export interface RetryState {
  attempt: number;
  reason: string;
}
const [retryMap, setRetryMap] = createStore<Record<string, RetryState | null>>({});
export function retryOf(id: string | null): RetryState | null {
  return id ? (retryMap[id] ?? null) : null;
}
function noteRetry(id: string, state: RetryState | null) {
  setRetryMap(id, state);
}

export interface UiPreferences {
  theme: "warm" | "dark" | "contrast";
  textScale: number;
  codeScale: number;
  compactSidebar: boolean;
  suggestions: boolean;
  notifications: boolean;
  reduceMotion: boolean;
}

const defaultUiPreferences: UiPreferences = {
  theme: "warm",
  textScale: 100,
  codeScale: 100,
  compactSidebar: false,
  suggestions: true,
  notifications: true,
  reduceMotion: false,
};

function loadUiPreferences(): UiPreferences {
  try {
    return { ...defaultUiPreferences, ...JSON.parse(localStorage.getItem("vakcoder.uiPreferences") ?? "{}") };
  } catch {
    return defaultUiPreferences;
  }
}

export const [uiPreferences, setUiPreferences] = createStore<UiPreferences>(loadUiPreferences());

export function updateUiPreference<K extends keyof UiPreferences>(key: K, value: UiPreferences[K]) {
  setUiPreferences(key, value);
  localStorage.setItem("vakcoder.uiPreferences", JSON.stringify({ ...uiPreferences, [key]: value }));
}
// File-editor pane target; set from anywhere (chat links, diff headers…).
export const [editorPath, setEditorPath] = createSignal<string | null>(null);
// `/btw` side chat panel.
export const [sideOpen, setSideOpen] = createSignal(false);
// Scheduled-tasks manager modal.
export const [tasksOpen, setTasksOpen] = createSignal(false);
// Cross-project recall search (docs/design/29-personal-os.md P1).
export const [searchOpen, setSearchOpen] = createSignal(false);
// Inbox page (docs/design/29-personal-os.md P6) + live unread total shared by
// the header bell and the sidebar badge.
export const [inboxOpen, setInboxOpen] = createSignal(false);
export const [inboxUnread, setInboxUnread] = createSignal(0);
// Settings page to land on when the next open happens (budget banner link).
export type SettingsPageId =
  | "general"
  | "appearance"
  | "agent"
  | "permissions"
  | "reliability"
  | "integrations"
  | "services"
  | "learning"
  | "advanced"
  | "archived";
export const [pendingSettingsPage, setPendingSettingsPage] = createSignal<SettingsPageId | null>(null);
// Read-only historical transcript viewer (docs/design/29): any session by id,
// served from disk — no attach, no stream, never touches live view state.
export const [transcriptViewId, setTranscriptViewId] = createSignal<string | null>(null);
// Time-travel (checkpoints) modal.
export const [historyOpen, setHistoryOpen] = createSignal(false);
// Dispatch-forensics (receipts) modal.
export const [receiptsOpen, setReceiptsOpen] = createSignal(false);
// Diff pane binding: which session's changes are shown (best-of-N override).
export const [diffTarget, setDiffTarget] = createSignal<string | null>(null);

export interface BestRun {
  session_id: string;
  branch: string;
  path: string;
}
// Dialog lifecycle: closed → config form → starting ([] pending) → runs.
export const [bestOfOpen, setBestOfOpen] = createSignal(false);
export const [bestOfRuns, setBestOfRuns] = createSignal<BestRun[] | null>(null);

export function openInEditor(path: string) {
  setEditorPath(path);
  setDockTab("editor");
}

const [itemsBySession, setItemsBySession] = createStore<Record<string, Item[]>>({});
const [runningMap, setRunningMap] = createStore<Record<string, boolean>>({});
const [usageBySession, setUsageBySession] = createStore<Record<string, Usage>>({});

// ---- selectors -------------------------------------------------------------

export function itemsOf(id: string | null, bucket: Bucket = "main"): Item[] {
  if (!id) return [];
  return itemsBySession[K(bucket, id)] ?? [];
}

export function isRunning(id: string | null, bucket: Bucket = "main"): boolean {
  return !!(id && runningMap[K(bucket, id)]);
}

export function usageOf(id: string | null): Usage {
  return (id && usageBySession[id]) || {};
}

// ---- buckets: "main" transcript vs "side" (/btw) branch --------------------

export type Bucket = "main" | "side";

const K = (bucket: Bucket, id: string) => (bucket === "main" ? id : `side:${id}`);

// ---- immutable list helpers ------------------------------------------------

function updateList(bucket: Bucket, id: string, fn: (list: Item[]) => Item[]) {
  setItemsBySession(K(bucket, id), (list) => fn(list ?? []));
}

function pushItem(bucket: Bucket, id: string, item: Item) {
  updateList(bucket, id, (list) => [...list, item]);
}

function note(bucket: Bucket, id: string, text: string) {
  pushItem(bucket, id, { kind: "system", text });
}

function patchById(bucket: Bucket, id: string, itemId: string, patch: (draft: Item) => Item) {
  updateList(bucket, id, (list) =>
    list.map((it) => {
      if ((it.kind === "tool" || it.kind === "approval") && it.id === itemId) {
        return patch(it);
      }
      return it;
    }),
  );
}

function patchLast(bucket: Bucket, id: string, pred: (it: Item) => boolean, patch: (draft: Item) => Item) {
  updateList(bucket, id, (list) => {
    for (let i = list.length - 1; i >= 0; i--) {
      if (pred(list[i])) {
        const next = [...list];
        next[i] = patch(list[i]);
        return next;
      }
    }
    return list;
  });
}

// ---- transcript hydration --------------------------------------------------

function blocksToItems(blocks: ContentBlock[], keyBase: string): Item[] {
  const out: Item[] = [];
  blocks.forEach((b, i) => {
    if (b.type === "text" && b.text.trim()) {
      out.push({ kind: "assistant", key: `${keyBase}-t${i}`, text: b.text, streaming: false });
    } else if (b.type === "thinking" && b.text.trim()) {
      out.push({ kind: "thinking", key: `${keyBase}-k${i}`, text: b.text, done: true });
    } else if (b.type === "tool_use") {
      out.push({
        kind: "tool",
        id: b.id,
        name: b.name,
        argsJson: JSON.stringify(b.input ?? null, null, 2),
        done: false,
        isError: false,
        preview: null,
      });
    }
    // tool_result blocks fold into their ToolUse card below.
  });
  return out;
}

/**
 * Rebuild chat items from a persisted ledger without writing any store
 * state. Shared by live hydration and the read-only transcript viewer.
 */
export function transcriptToItems(id: string, messages: Message[]): Item[] {
  const next: Item[] = [];
  let assistantSeq = 0;

  for (const m of messages) {
    if (m.role === "User") {
      const texts = m.content
        .filter((b): b is Extract<ContentBlock, { type: "text" }> => b.type === "text")
        .map((b) => b.text);
      const results = m.content.filter(
        (b): b is Extract<ContentBlock, { type: "tool_result" }> => b.type === "tool_result",
      );
      for (const tr of results) {
        const idx = next.findIndex(
          (it) => it.kind === "tool" && it.id === tr.tool_use_id,
        );
        if (idx >= 0) {
          const it = next[idx];
          if (it.kind === "tool") {
            next[idx] = { ...it, done: true, isError: !!tr.is_error, preview: tr.content };
          }
        }
      }
      const joined = texts.join("\n").trim();
      if (joined) next.push({ kind: "user", text: joined });
    } else {
      const baseKey = `${id}-h${assistantSeq++}`;
      const hasText = m.content.some(
        (b): b is Extract<ContentBlock, { type: "text" }> =>
          b.type === "text" && b.text.trim().length > 0,
      );
      next.push(...blocksToItems(m.content, baseKey));
      if (hasText) {
        for (let i = next.length - 1; i >= 0; i--) {
          const it = next[i];
          if (it.kind === "assistant") {
            next[i] = { ...it, streaming: false };
            break;
          }
        }
      }
    }
  }
  // Nothing here can still be in flight: hydration only runs for a session
  // that is not currently running, so a tool without a recorded result had
  // its end event lost rather than being genuinely mid-execution.
  for (let i = 0; i < next.length; i++) {
    const it = next[i];
    if (it.kind === "tool" && !it.done) {
      next[i] = { ...it, done: true, preview: it.preview ?? null };
    }
    if (it.kind === "approval" && it.resolved === null) {
      next[i] = { ...it, resolved: "gone" };
    }
  }
  return next;
}

/** Rebuild a session view from the persisted ledger. */
export function hydrateFromTranscript(id: string, messages: Message[]) {
  setItemsBySession(id, transcriptToItems(id, messages));
}

// ---- live event application ------------------------------------------------

let assistantSeq = 0;

function ensureStreamingAssistant(bucket: Bucket, id: string): void {
  updateList(bucket, id, (list) => {
    for (let i = list.length - 1; i >= 0; i--) {
      const it = list[i];
      if (it.kind === "assistant") return list;
      if (it.kind === "user") break;
    }
    assistantSeq += 1;
    return [
      ...list,
      { kind: "assistant", key: `live-${assistantSeq}`, text: "", streaming: true },
    ];
  });
}

function appendToLast(bucket: Bucket, id: string, kind: "assistant" | "thinking", delta: string) {
  updateList(bucket, id, (list) => {
    for (let i = list.length - 1; i >= 0; i--) {
      const it = list[i];
      if (it.kind === kind) {
        const next = [...list];
        next[i] =
          kind === "assistant"
            ? { ...it, kind, key: it.key, text: it.text + delta, streaming: true }
            : { ...it, kind, key: it.key, text: it.text + delta, done: false };
        return next;
      }
      // stop scanning at the last hard boundary
      if (it.kind === "user" || it.kind === "system") break;
    }
    const fresh: Item =
      kind === "assistant"
        ? { kind: "assistant", key: `live-${++assistantSeq}`, text: delta, streaming: true }
        : { kind: "thinking", key: `think-${Date.now()}`, text: delta, done: false };
    return [...list, fresh];
  });
}

export function applyEvent(
  id: string,
  ev: AgentEvent,
  opts: { onFinish?: (summary: string) => void; bucket?: Bucket },
) {
  const b: Bucket = opts.bucket ?? "main";
  if ("TurnStart" in ev) {
    markRunning(id, true, b);
  } else if ("Stream" in ev) {
    const s = ev.Stream;
    if ("TextDelta" in s) {
      ensureStreamingAssistant(b, id);
      appendToLast(b, id, "assistant", s.TextDelta.delta);
    } else if ("ThinkingDelta" in s) {
      appendToLast(b, id, "thinking", s.ThinkingDelta.delta);
    } else if ("End" in s) {
      finalizeStream(b, id);
    }
  } else if ("ToolCallStart" in ev) {
    pushItem(b, id, {
      kind: "tool",
      id: ev.ToolCallStart.id,
      name: ev.ToolCallStart.name,
      argsJson: ev.ToolCallStart.args_json,
      done: false,
      isError: false,
      preview: null,
    });
  } else if ("ToolCallEnd" in ev) {
    patchById(b, id, ev.ToolCallEnd.id, (it) =>
      it.kind === "tool"
        ? {
            ...it,
            done: true,
            isError: ev.ToolCallEnd.is_error,
            preview: ev.ToolCallEnd.result_preview,
          }
        : it,
    );
  } else if ("ApprovalRequested" in ev) {
    pushItem("main", id, {
      kind: "approval",
      id: ev.ApprovalRequested.id,
      tool: ev.ApprovalRequested.tool,
      argsJson: ev.ApprovalRequested.args_json,
      reason: ev.ApprovalRequested.reason,
      resolved: null,
    });
  } else if ("SubagentStarted" in ev) {
    pushItem(b, id, {
      kind: "subagent",
      label: ev.SubagentStarted.label,
      lines: [],
      open: false,
      isError: false,
    });
  } else if ("SubagentToolCall" in ev) {
    patchLast(
      b,
      id,
      (it) => it.kind === "subagent",
      (it) => {
        if (it.kind !== "subagent") return it;
        const lines = [...it.lines, `${ev.SubagentToolCall.name}${ev.SubagentToolCall.is_error ? " ✗" : ""}`];
        return { ...it, lines: lines.slice(-12) };
      },
    );
  } else if ("SubagentUsage" in ev) {
    patchLast(
      b,
      id,
      (it) => it.kind === "subagent",
      (it) =>
        it.kind === "subagent"
          ? {
              ...it,
              lines: [...it.lines, `tokens ↑${ev.SubagentUsage.input_tokens} ↓${ev.SubagentUsage.output_tokens}`],
            }
          : it,
    );
  } else if ("SubagentFinished" in ev) {
    patchLast(
      b,
      id,
      (it) => it.kind === "subagent",
      (it) =>
        it.kind === "subagent"
          ? {
              ...it,
              isError: ev.SubagentFinished.is_error,
              lines: [...it.lines, `done in ${(ev.SubagentFinished.elapsed_ms / 1000).toFixed(1)}s`],
            }
          : it,
    );
  } else if ("RetryScheduled" in ev) {
    noteRetry(id, {
      attempt: ev.RetryScheduled.attempt,
      reason: ev.RetryScheduled.reason,
    });
    note(
      b,
      id,
      `retrying (attempt ${ev.RetryScheduled.attempt}) in ${Math.round(ev.RetryScheduled.delay_ms / 100) / 10}s — ${ev.RetryScheduled.reason}`,
    );
  } else if ("RouteFallback" in ev) {
    // The leg changed: whatever backoff the previous leg scheduled no
    // longer describes this moment. Clear the header banner and show the
    // frozen-contract step instead.
    noteRetry(id, null);
    note(
      b,
      id,
      `route fallback → ${ev.RouteFallback.to_provider}/${ev.RouteFallback.to_model} (frozen ladder leg)`,
    );
  } else if ("ContextCompacting" in ev) {
    note(b, id, "compacting context…");
  } else if ("ContextCompacted" in ev) {
    note(
      b,
      id,
      `context compacted ${ev.ContextCompacted.before_tokens} → ${ev.ContextCompacted.after_tokens} tokens`,
    );
  } else if ("StopHookContinuation" in ev) {
    note(b, id, `stop gate: continuing (${ev.StopHookContinuation.reason})`);
  } else if ("TurnEnd" in ev) {
    setUsageBySession(id, ev.TurnEnd.usage);
  } else if ("RunFinished" in ev) {
    markRunning(id, false, b);
    // Close every in-flight item. A finished run cannot still have a tool
    // executing or an approval pending, so anything left open means its
    // end event was missed (a dropped stream, a failed re-attach). Leaving
    // it open strands the card on "running…" for the rest of the session.
    updateList(b, id, (list) =>
      list.map((it) => {
        if (it.kind === "assistant" && it.streaming) return { ...it, streaming: false };
        if (it.kind === "thinking" && !it.done) return { ...it, done: true };
        if (it.kind === "tool" && !it.done) {
          return {
            ...it,
            done: true,
            preview: it.preview ?? null,
          };
        }
        if (it.kind === "approval" && it.resolved === null) {
          return { ...it, resolved: "gone" as const };
        }
        return it;
      }),
    );
    // A failed run must say so in the transcript. Previously the reason
    // lived only in the summary handed to onFinish, which surfaced it only
    // as a background notification — so a visible window showed nothing at
    // all when a run died.
    if (ev.RunFinished.is_error) {
      note(b, id, ev.RunFinished.summary);
    }
    opts.onFinish?.(ev.RunFinished.summary);
  }
}

function finalizeStream(bucket: Bucket, id: string) {
  // The final snapshot may still be mid-flight deltas; closing streaming
  // state happens on RunFinished or when the next user turn starts.
}

// ---- imperative helpers used by App ---------------------------------------

export function appendUser(id: string, text: string, bucket: Bucket = "main") {
  pushItem(bucket, id, { kind: "user", text });
}

export function appendSystem(id: string, text: string, bucket: Bucket = "main") {
  note(bucket, id, text);
}

export function markRunning(id: string, on: boolean, bucket: Bucket = "main") {
  setRunningMap(K(bucket, id), on);
}

export function resolveApproval(id: string, requestId: string, verdict: "allowed" | "denied") {
  patchById(
    "main",
    id,
    requestId,
    (it): Item =>
      it.kind === "approval" ? { ...it, resolved: verdict } : it,
  );
}

export function setUsageFor(id: string, usage: Usage | Record<string, number>) {
  setUsageBySession(id, usage as Usage);
}

// ---- session registry ------------------------------------------------------

export function resetSessionView(id: string) {
  for (const key of [K("main", id), K("side", id)]) {
    setItemsBySession(key, []);
    setRunningMap(key, false);
  }
}
