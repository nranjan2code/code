import { createEffect, createMemo, createSignal, For, Index, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { activeId, density, itemExpanded, itemsOf, hydratingId, isRunning, presentationOf, uiPreferences, openWorkbenchExecution, presentationMode, setNotice, toggleItemExpanded, type Item } from "../store";
import { approve, isApprovalPending, openFileSmart } from "../App";
import Icon from "./Icon";
import MarkdownView from "./MarkdownView";
import MessageActions from "./MessageActions";
import PresentationTimelineView, { StructuredView } from "./PresentationRenderer";
import * as api from "../api";
import "../focusTrap";
import { assistantParts, cleanAssistantText, parseVakFence, stripControlScaffolding } from "../structured";
export { parseVakFence, stripControlScaffolding };

/// The typed-output transport fence: a ` ```vak ``` ` block in a tool
function stripVakFence(text: string): string {
  let cleaned = stripControlScaffolding(text);
  cleaned = cleaned.replace(/```(?:vak|json)?\s*\{[\s\S]*?"semantic_type"[\s\S]*?\}\s*```/gi, "");
  cleaned = cleaned.replace(/```vak\s*[\s\S]*?(?:```|$)/gi, "");
  const trimmed = cleaned.trim();
  if (!trimmed && text.includes('"semantic_type"')) {
    return "Structured output rendered in the presentation timeline.";
  }
  return trimmed || text;
}


/**
 * A new task's chat pane before anything has happened, and an existing
 * task with nothing rendered at the current density. Previously both
 * rendered `null` — a void with no headline, hint, or affordance —
 * despite DESIGN.md naming "the chat empty state" as the canonical use
 * of the headline type scale (22px/620/-0.02em) it defines.
 */
function EmptyChat(props: { hasSession: boolean }) {
  return (
    <div class="chat-empty">
      <div class="chat-empty-mark">
        <Icon name="chat" size={22} />
      </div>
      <h2 class="chat-empty-headline">
        {props.hasSession ? "Nothing here yet" : presentationMode() === "everyday" ? "What would you like to do?" : "Start a task"}
      </h2>
      <p class="chat-empty-hint">
        {props.hasSession
          ? "This task has no visible activity at the current transcript detail. Switch to \"balanced\" or \"audit\" in the composer to see more."
          : "Ask Vak to build, fix, explain, research, write, or analyze something — it starts a task with this workspace's files and history."}
      </p>
      <Show when={!props.hasSession && presentationMode() === "everyday"}>
        <div class="chat-empty-examples" aria-label="Things Vak can help with">
          <For each={[
            ["Research a question", "Research this question and summarize the important points."],
            ["Write or rewrite", "Help me write or rewrite this clearly: "],
            ["Analyze data", "Help me analyze this data and explain the key findings."],
            ["Plan something", "Help me make a practical plan for: "],
          ]}>
            {([label, prompt]) => (
              <button
                type="button"
                class="chat-empty-example"
                onClick={() => window.dispatchEvent(new CustomEvent("vak:edit-prompt", { detail: { text: prompt } }))}
              >
                {label}
              </button>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

/** True while a turn is running but nothing currently *visible at this
 * density* shows its own activity — the model is between tokens/tool
 * calls (or, in "outcome" density, thinking/working on a tool that
 * density hides) with literally nothing animating on screen. This is
 * the gap that otherwise reads as a dead, stuck UI.
 *
 * Checked against the density-filtered list, not the raw item list:
 * "outcome" hides thinking and completed tool cards, so a raw-list check
 * could see a live thinking item and suppress the indicator while the
 * screen itself shows nothing at all. */
function awaitingNextOutput(id: string | null): boolean {
  if (!isRunning(id)) return false;
  const list = visibleItems(itemsOf(id));
  const last = list[list.length - 1];
  if (!last) return true;
  if (last.kind === "assistant" && last.streaming) return false;
  if (last.kind === "thinking" && !last.done) return false;
  if (last.kind === "tool" && !last.done) return false;
  return true;
}

function ThinkingIndicator() {
  return (
    <div class="thinking-row" aria-live="polite" aria-label="Working">
      <span class="thinking-dots">
        <span /><span /><span />
      </span>
    </div>
  );
}

function TranscriptSkeleton() {
  return (
    <div class="transcript-skeleton" aria-label="Loading task">
      <span class="skeleton-line wide" />
      <span class="skeleton-line medium" />
      <span class="skeleton-card" />
      <span class="skeleton-line wide" />
      <span class="skeleton-line short" />
    </div>
  );
}

function visibleItems(list: Item[]): Item[] {
  // Filter out any control scaffolding messages (e.g. <conversation_thread>,
  // <context_summary>, <intent>, <work_contract>) so they never leak into the chat canvas.
  const cleanList = list.filter((it) => {
    if (it.kind === "user") {
      return stripControlScaffolding(it.text).length > 0;
    }
    if (it.kind === "assistant") {
      if (it.streaming) return true;
      const scrubbed = cleanAssistantText(it.text);
      if (!scrubbed.trim()) {
        return false;
      }
      return true;
    }
    return true;
  });

  // Everyday is the calm product surface: it never exposes the verbose
  // activity ledger, even when a previous Advanced session left Audit
  // selected in local storage. Advanced remains the operator surface where
  // the stored transcript detail preference is honored.
  const d = presentationMode() === "everyday" ? "outcome" : density();
  if (d === "outcome") {
    // "Outcome" hides the working (thinking, tool-call) detail once it's
    // done — but a run in progress must still show *something* live, or
    // the pane reads as frozen for the entire stretch between the last
    // settled turn and this one's reply. A streaming assistant reply
    // (the outcome, forming) and the single most recent in-flight tool
    // call are the two forms "current activity" can take here; both
    // disappear from this density the moment they settle, exactly as
    // before.
    return cleanList.filter((it, i) => {
      if (it.kind === "user") return true;
      if (it.kind === "system") {
        const t = it.text.toLowerCase();
        if (
          t.includes("compacting context") ||
          t.includes("context compacted") ||
          t.includes("route fallback") ||
          t.includes("stop gate:")
        ) {
          return false;
        }
        return true;
      }
      if (it.kind === "approval" && !it.resolved) return true;
      if (it.kind === "assistant") {
        if (it.streaming) return true;
        const scrubbed = cleanAssistantText(it.text);
        return Boolean(scrubbed.trim());
      }
      if (it.kind === "tool" && !it.done && i === cleanList.length - 1) return true;
      return false;
    });
  }
  return cleanList;
}

function RunControls(props: { sessionId: string }) {
  const [paused, setPaused] = createSignal(false);
  const [revision, setRevision] = createSignal(0);
  const [busy, setBusy] = createSignal(false);
  const [controlError, setControlError] = createSignal("");
  const [changing, setChanging] = createSignal(false);
  const [changeText, setChangeText] = createSignal("");
  const [changeKind, setChangeKind] = createSignal("replan");
  const [changeResult, setChangeResult] = createSignal("");
  const refresh = () => void api.controlState(props.sessionId).then((state) => {
    setPaused(state.paused);
    setRevision(state.revision);
    setControlError("");
  }).catch((error) => {
    setControlError(`Control state unavailable: ${error instanceof Error ? error.message : String(error)}`);
  });
  onMount(() => {
    refresh();
    const timer = window.setInterval(refresh, 2000);
    onCleanup(() => window.clearInterval(timer));
  });
  const toggle = async () => {
    if (busy()) return;
    setBusy(true);
    try {
      if (paused()) await api.resumeRun(props.sessionId);
      else await api.pauseRun(props.sessionId);
      // Read back the control state so the label reflects the server's
      // revision rather than a local optimistic guess.
      await api.controlState(props.sessionId).then((state) => {
        setPaused(state.paused);
        setRevision(state.revision);
      });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not ${paused() ? "resume" : "pause"} this task: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setBusy(false);
    }
  };
  const submitChange = async () => {
    const text = changeText().trim();
    if (!text || busy()) return;
    setBusy(true);
    try {
      const result = await api.planChange(props.sessionId, `${changeKind()}: ${text}`, "human", revision());
      if (result.decision === "requires_human") {
        setChangeResult(`Human review required · plan v${result.revision}`);
      } else {
        setRevision(result.revision);
        setChangeResult(`${result.decision} · plan v${result.revision}`);
      }
      setChangeText("");
    } catch (error) {
      const message = error instanceof Error ? error.message : "Plan change failed";
      if (message.includes("409")) {
        refresh();
        setChangeResult("Plan changed elsewhere; review the new revision");
      } else {
        setChangeResult(message);
      }
    } finally {
      setBusy(false);
    }
  };
  return <Show when={isRunning(props.sessionId)}>
    <div class="run-controls" aria-label="Live task controls">
      <button type="button" class="run-control" disabled={busy()} onClick={() => void toggle()}>{paused() ? "Resume" : "Pause"}</button>
      <button type="button" class="run-control danger" disabled={busy()} onClick={() => void api.cancelRun(props.sessionId).catch((error) => setNotice({ kind: "error", text: `Could not cancel this task: ${error instanceof Error ? error.message : String(error)}` }))}>Cancel</button>
      <button type="button" class="run-control" disabled={busy()} onClick={() => setChanging(!changing())}>Change plan</button>
      <span class="run-revision" title="Active outcome plan revision">Plan v{revision()}</span>
      <Show when={paused()}><span class="run-paused" role="status">Paused at safe boundary</span></Show>
    </div>
    <Show when={controlError()}>
      <div class="inline-error" role="alert">{controlError()} Refreshing will retry; the server remains authoritative.</div>
    </Show>
    <Show when={changing()}>
      <form class="plan-change" onSubmit={(event) => { event.preventDefault(); void submitChange(); }}>
        <select aria-label="Plan change type" value={changeKind()} onChange={(event) => setChangeKind(event.currentTarget.value)}>
          <option value="replan">Replan</option>
          <option value="add requirement">Add requirement</option>
          <option value="remove requirement">Remove requirement</option>
          <option value="reprioritize">Reprioritize</option>
        </select>
        <input aria-label="Plan change" value={changeText()} placeholder="Add, remove, or reprioritize work…" onInput={(event) => setChangeText(event.currentTarget.value)} />
        <button class="run-control" type="submit" disabled={busy() || !changeText().trim()}>Submit</button>
        <Show when={changeResult()}><span role="status">{changeResult()}</span></Show>
      </form>
    </Show>
  </Show>;
}

export const ToolCard = (props: { item: Extract<Item, { kind: "tool" }> }) => {
  const open = () => itemExpanded(props.item.id);
  const args = createMemo<Record<string, unknown> | null>(() => {
    try {
      const v = JSON.parse(props.item.argsJson);
      return v && typeof v === "object" ? (v as Record<string, unknown>) : null;
    } catch {
      return null;
    }
  });
  const argsPretty = createMemo(() => {
    const a = args();
    return a ? JSON.stringify(a, null, 2) : props.item.argsJson;
  });

  /** The file this call acts on, when it names one. */
  const filePath = createMemo(() => {
    const p = args()?.path ?? args()?.file_path;
    return typeof p === "string" && p ? p : null;
  });
  const shortPath = () => {
    const p = filePath();
    if (!p) return null;
    // Workspace-relative reads better than an absolute path in a card.
    const parts = p.split("/").filter(Boolean);
    return parts.length > 2 ? parts.slice(-2).join("/") : p;
  };
  const toolLabel = () => ({
    read_file: "Read file",
    write_file: "Write file",
    edit_file: "Edit file",
    bash: "Run command (Sandbox)",
    grep: "Search files",
    glob: "Find files",
    mcp: "Use connected tool",
  } as Record<string, string>)[props.item.name] ?? props.item.name.replaceAll("_", " ");
  const summary = createMemo(() => {
    const a = args();
    if (!a) return null;
    if (props.item.name === "mcp") {
      const server = typeof a.server === "string" ? a.server : "connected service";
      const tool = typeof a.tool === "string" ? a.tool : "tool";
      return `${server} · ${tool}`;
    }
    if (props.item.name === "bash" && typeof a.command === "string") {
      return a.command.replace(/\s+/g, " ").trim().slice(0, 120);
    }
    return null;
  });
  const status = () => props.item.isError ? "Failed" : props.item.done ? "Completed" : "Running";
  const result = () => {
    const value = props.item.preview?.trim();
    if (value) {
      const shown = stripVakFence(value);
      return shown.slice(0, density() === "audit" ? 4000 : 800);
    }
    return props.item.done ? "No output returned." : "Waiting for a result…";
  };


  return (
      <div class="tool" classList={{ err: props.item.isError, open: open(), running: !props.item.done, done: props.item.done }}>
      <button type="button" class="tool-h" aria-expanded={open()} onClick={() => toggleItemExpanded(props.item.id)}>
        <span class="tool-dot" />
        <span class="tool-name">{toolLabel()}</span>
        <Show when={summary()}>{(value) => <span class="tool-summary">{value()}</span>}</Show>
        <Show when={shortPath()}>
          <span class="tool-path">{shortPath()}</span>
        </Show>
        <span class="tool-state">{status()}</span>
        <span class="tool-chev"><Icon name="chevron" size={14} /></span>
      </button>
      <Show when={filePath()}>
        {(path) => (
          <div class="tool-actions">
            {/* One action, routed for you: a changed file opens as a diff,
                a new one opens in the editor. Inline dumps do not scale
                past the first file. */}
            <button type="button" class="tool-open" onClick={() => void openFileSmart(path())} title={path()}>
              <Icon name="code" size={12} /> View file
            </button>
          </div>
        )}
      </Show>
      <Show when={props.item.name === "bash"}>
        <div class="tool-actions">
          <button
            class="tool-open"
            onClick={() => {
              openWorkbenchExecution();
            }}
            title="Inspect execution in Workbench Sandbox"
          >
            <Icon name="terminal" size={12} /> Inspect in Workbench
          </button>
        </div>
      </Show>
      <Show when={open() || density() === "audit" || props.item.isError || !props.item.done}>
        <div class="tool-result" classList={{ err: props.item.isError }}>{result()}</div>
      </Show>
      <details class="tool-details" open={open() || density() === "audit"}>
        <summary>View request details</summary>
        <pre class="tool-args">{argsPretty()}</pre>
      </details>
    </div>
  );
};

/** Arg keys that name the subject of a webfetch-style tool call. */
const APPROVAL_PRIMARY_KEYS = ["url", "path", "file_path", "command", "file", "dir"] as const;

const ApprovalCard = (props: { item: Extract<Item, { kind: "approval" }>; sessionId?: string | null }) => {
  const [showRulePreview, setShowRulePreview] = createSignal(false);
  // Webfetch-style tools name their target under different keys; whatever
  // the tool calls its subject (url/path/command…) is what the user needs
  // to see before deciding, so it gets the prominent slot.
  const primary = createMemo<{ key: string; value: string } | null>(() => {
    try {
      const parsed = JSON.parse(props.item.argsJson) as Record<string, unknown>;
      for (const key of APPROVAL_PRIMARY_KEYS) {
        const value = parsed[key];
        if (typeof value === "string" && value.trim()) {
          return { key, value: value.trim() };
        }
      }
    } catch {
      /* malformed args fall through to the generic summary below */
    }
    return null;
  });
  const summary = createMemo(() => {
    try {
      const parsed = JSON.parse(props.item.argsJson) as Record<string, unknown>;
      return Object.entries(parsed)
        .filter(([, value]) => typeof value === "string" && value.length < 180)
        .slice(0, 2)
        .map(([key, value]) => `${key.replaceAll("_", " ")}: ${String(value)}`)
        .join(" · ");
    } catch {
      return "Review the requested operation before allowing it.";
    }
  });
  return (
    <div class="approval" data-approval={props.item.id} role={props.item.resolved ? "status" : "alert"} aria-live={props.item.resolved ? "polite" : "assertive"} aria-label={`${props.item.resolved ? "Approval resolved" : "Approval requested"} for ${props.item.tool}`}>
    <div class="ap-head">Approval requested — {props.item.tool}</div>
    <Show when={primary()}>
      {(p) => (
        <code class="ap-primary" title={p().value}>
          <span class="ap-primary-key">{p().key}</span>
          {p().value}
        </code>
      )}
    </Show>
    <Show when={props.item.reason}>
      <div class="ap-reason">{props.item.reason}</div>
    </Show>
    <div class="ap-summary">{summary()}</div>
    <details class="ap-details">
      <summary>View request details</summary>
      <pre class="ap-args">{props.item.argsJson}</pre>
    </details>
    <Show
      when={!props.item.resolved}
      fallback={<div class="ap-done">{props.item.resolved}</div>}
    >
      <div class="ap-actions" aria-busy={isApprovalPending(props.item.id)}>
        <button type="button" class="btn primary" disabled={isApprovalPending(props.item.id)} onClick={() => void approve(props.item.id, true, props.sessionId)}>
          {isApprovalPending(props.item.id) ? "Allowing…" : "Allow once"}
        </button>
        <div class="ap-rule-hint" role="note">
          Creates a persistent rule for this workspace. You can revoke it later in Settings → Permissions.
        </div>
        <button
          type="button"
          class="btn"
          title="Create a persistent permission rule for this workspace"
          disabled={isApprovalPending(props.item.id)}
          onClick={() => setShowRulePreview(true)}
        >
          Create rule…
        </button>
        <button type="button" class="btn danger" disabled={isApprovalPending(props.item.id)} onClick={() => void approve(props.item.id, false, props.sessionId)}>
          {isApprovalPending(props.item.id) ? "Resolving…" : "Deny"}
        </button>
      </div>
    </Show>
    <Show when={showRulePreview()}>
      <div class="modal-back" onClick={() => setShowRulePreview(false)}>
        <div class="modal confirm-modal ap-rule-modal" role="dialog" aria-modal="true" aria-labelledby={`rule-title-${props.item.id}`} onClick={(e) => e.stopPropagation()} use:trapFocus>
          <h3 id={`rule-title-${props.item.id}`}>Create persistent rule?</h3>
          <p>This rule will apply automatically to matching requests in this workspace.</p>
          <dl class="ap-rule-preview">
            <div><dt>Matcher</dt><dd><code>{props.item.tool}</code></dd></div>
            <div><dt>Workspace</dt><dd>{props.sessionId || "Current workspace"}</dd></div>
            <div><dt>Effect</dt><dd>Allow this request pattern</dd></div>
            <div><dt>Revoke</dt><dd>Settings → Permissions → Rules</dd></div>
          </dl>
          <div class="confirm-modal-actions">
            <button type="button" class="btn-subtle" onClick={() => setShowRulePreview(false)}>Cancel</button>
            <button type="button" class="btn-action" onClick={() => { setShowRulePreview(false); void approve(props.item.id, true, props.sessionId, true); }}>Create rule</button>
          </div>
        </div>
      </div>
    </Show>
  </div>
  );
};

/**
 * Replace each fenced block's plain text with highlighted markup in place,
 * leaving the header and copy button (which holds the raw text) untouched.
 */
export const Markdown = MarkdownView;

/**
 * One transcript row, shared by the live chat and the read-only historical
 * viewer. Items are immutable snapshots replaced by identity in the store,
 * so `<For>` re-creates a row whenever its item changes and a plain read
 * here is safe.
 */
export const ItemView = (props: { item: Item; sessionId?: string | null }): JSX.Element => {
  const item = props.item;
  if (item.kind === "user") {
    const text = stripControlScaffolding(item.text);
    if (!text) return null;
    return (
      <div class="msg user">
        <div class="msg-bubble-wrap">
          <div class="user-turn-head">
            <span class="turn-author-chip">You</span>
          </div>
          <div class="msg-user-content"><Markdown text={text} /></div>
          <MessageActions text={text} role="user" />
        </div>
      </div>
    );
  }
  if (item.kind === "assistant") return <AssistantItem item={item} sessionId={props.sessionId} />;
  if (item.kind === "thinking") {
    return (
      <details class="thinking">
        <summary class="thinking-summary">
          <span class="thinking-sparkle"><Icon name="spark" size={13} /></span>
          <span>{item.done ? "Thought process" : "Thinking…"}</span>
        </summary>
        <pre class="thinking-pre">{item.text}</pre>
      </details>
    );
  }
  if (item.kind === "tool") {
    return <ToolCard item={item} />;
  }
  if (item.kind === "approval") {
    return <ApprovalCard item={item} sessionId={props.sessionId} />;
  }
  if (item.kind === "subagent") {
    return (
      <details class="subagent">
        <summary>
          subagent · {item.label}
          {item.isError ? " ✗" : ""}
        </summary>
        <pre>{item.lines.join("\n")}</pre>
      </details>
    );
  }
  return <div class="sysnote">{item.text}</div>;
};

function AssistantItem(props: { item: Extract<Item, { kind: "assistant" }>; sessionId?: string | null }) {
    const [content, setContent] = createStore<{ parts: ReturnType<typeof assistantParts> }>({ parts: [] });
    createEffect(() => setContent("parts", reconcile(assistantParts(props.item.text, props.item.streaming), { key: null })));
    const parts = () => content.parts;
    const displayText = () => parts().filter((part) => part.type === "text").map((part) => part.text).join("\n\n");


    return (
      <div class="msg assistant">
        <div class="assistant-turn-head">
          <span class="assistant-avatar-mark">
            <img src={`${import.meta.env.BASE_URL}vak-icon.png`} alt="" class="assistant-avatar-img" />
          </span>
          <span class="assistant-name">Vak</span>
          <Show when={props.item.streaming}>
            <span class="assistant-live-pulse" title="Generating">
              <span class="dot run" />
            </span>
          </Show>
        </div>
        <div class="assistant-turn-body">
          <For each={parts()}>{(part) => part.type === "text"
            ? <Markdown text={part.text} streaming={props.item.streaming} />
            : <StructuredView output={part.output} fallback={part.source} sessionId={props.sessionId ?? undefined} />}</For>
          <Show when={props.item.streaming && !displayText()}>
            <span class="caret" />
          </Show>
          <Show when={!props.item.streaming && displayText()}>
            <MessageActions text={displayText()} role="assistant" />
          </Show>
        </div>
      </div>
    );
}

export default function ChatPane(props: { sessionId?: string | null }) {
  // In split view each pane renders ITS OWN session; without the prop the
  // pane follows the global focus (previous behavior, unchanged).
  const sid = () => props.sessionId ?? activeId();
  let scroller!: HTMLDivElement;
  let content!: HTMLDivElement;
  let pinned = true;
  let scrollFrame: number | null = null;
  let smoothScrolling = false;
  let smoothTimer: number | null = null;
  const [atBottom, setAtBottom] = createSignal(true);

  const onScroll = () => {
    if (smoothScrolling) return;
    pinned = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
    setAtBottom(pinned);
  };
  const scrollToBottom = (force = false) => {
    if (force) {
      smoothScrolling = true;
      if (smoothTimer !== null) clearTimeout(smoothTimer);
      smoothTimer = window.setTimeout(() => { smoothScrolling = false; }, 400);
      scroller.scrollTo({ top: scroller.scrollHeight, behavior: uiPreferences.reduceMotion || window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
      pinned = true;
      setAtBottom(true);
      return;
    }
    if (pinned && !smoothScrolling) {
      scroller.scrollTo({ top: scroller.scrollHeight, behavior: "auto" });
      pinned = true;
      setAtBottom(true);
    }
  };
  const scheduleScroll = (force = false) => {
    if (force) {
      if (scrollFrame !== null) cancelAnimationFrame(scrollFrame);
      scrollFrame = null;
      scrollToBottom(true);
      return;
    }
    if (scrollFrame !== null) return;
    scrollFrame = requestAnimationFrame(() => {
      scrollFrame = null;
      scrollToBottom();
    });
  };

  createEffect(() => {
    const id = sid();
    void id;
    pinned = true;
    setAtBottom(true);
    queueMicrotask(() => scheduleScroll());
  });

  createEffect(() => {
    itemsOf(sid()).length;
    scheduleScroll();
  });

  // follow streaming text growth too
  createEffect(() => {
    const list = itemsOf(sid());
    const last = list[list.length - 1];
    if (last?.kind === "assistant") void last.text;
    scheduleScroll();
  });

  createEffect(() => {
    // Density changes alter transcript layout. Reconcile the pinned state on next paint.
    void density();
    scheduleScroll();
  });

  onMount(() => {
    // Keep scroller pinned when async markdown/code block syntax highlighting updates the DOM
    const observer = new MutationObserver(() => {
      if (pinned) {
        scheduleScroll();
      }
    });
    observer.observe(scroller, { childList: true, subtree: true, characterData: true });
    const resize = new ResizeObserver(() => { if (pinned) scheduleScroll(); });
    resize.observe(content);
    onCleanup(() => { observer.disconnect(); resize.disconnect(); });
  });

  onCleanup(() => {
    if (scrollFrame !== null) cancelAnimationFrame(scrollFrame);
    if (smoothTimer !== null) clearTimeout(smoothTimer);
  });
  const turns = createMemo(() => {
    const grouped: Item[][] = [[]];
    for (const item of itemsOf(sid())) {
      if (item.kind === "user") grouped.push([]);
      grouped[grouped.length - 1].push(item);
    }
    return grouped;
  });
  const projectedTurn = (index: number) => {
    const timeline = presentationOf(sid());
    if (!timeline) return null;
    const items = timeline.items.filter((item) => item.turn_id === `turn-${index}`);
    // Live frames have separate IDs. A durable turn replaces its transcript
    // only when its assistant output is available, never at RunFinished alone.
    if (!items.some((item) => item.role === "assistant" && ["document", "structured", "adaptive"].includes(item.content.type))) return null;
    if (index === turns().length - 1 && isRunning(sid())) return null;
    return { ...timeline, items };
  };
  return (
    <div class="chat-shell">
      <Show when={sid()}>{(id) => <RunControls sessionId={id()} />}</Show>
      <div class="chat" ref={scroller} onScroll={onScroll}>
        <div ref={content}>
        <Show when={sid()} fallback={<EmptyChat hasSession={false} />}>
          <Show when={hydratingId() !== sid() || itemsOf(sid()).length > 0} fallback={<TranscriptSkeleton />}>
            {/* Unified continuous chat canvas: The transcript stays permanently mounted
                across live and settled states so streaming cards, settled cards, approvals,
                and message actions maintain an unbroken, flicker-free rendering lifecycle. */}
            <Show when={visibleItems(itemsOf(sid())).length || awaitingNextOutput(sid())} fallback={<EmptyChat hasSession={true} />}>
              <Index each={turns()}>{(turn, index) =>
                <Show when={projectedTurn(index)} fallback={<Index each={visibleItems(turn())}>{(it) => <Show when={it().kind === "assistant"} fallback={<For each={[it()]}>{(item) => <ItemView item={item} sessionId={sid()} />}</For>}><AssistantItem item={it() as Extract<Item, { kind: "assistant" }>} sessionId={sid()} /></Show>}</Index>}>
                  {(timeline) => <PresentationTimelineView timeline={timeline()} sessionId={sid()!} />}
                </Show>
              }</Index>
              <Show when={awaitingNextOutput(sid())}><ThinkingIndicator /></Show>
            </Show>
          </Show>
        </Show>
        </div>
      </div>
      <Show when={!atBottom()}>
        <button type="button" class="scroll-latest" onClick={() => scrollToBottom(true)}>
          <Icon name="chevron" size={13} /> Latest
        </button>
      </Show>
    </div>
  );
}
