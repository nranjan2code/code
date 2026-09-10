import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import { activeId, density, itemExpanded, itemsOf, hydratingId, isRunning, openComponentPreview, openWorkbenchExecution, openInEditor, presentationMode, presentationOf, setNotice, toggleItemExpanded, type Item } from "../store";
import { approve, isApprovalPending, openFileSmart } from "../App";
import Icon from "./Icon";
import PresentationTimelineView from "./PresentationRenderer";
import MarkdownView from "./MarkdownView";
import * as api from "../api";
import "../focusTrap";

/// The typed-output transport fence: a ` ```vak ``` ` block in a tool
/// result carries structured envelope JSON the presentation timeline renders
/// as a card. The legacy tool row strips it so the same call is not shown
/// twice (raw JSON here, card above) — the pre-fence human summary stays.
function stripVakFence(text: string): string {
  const fence = text.indexOf("```vak");
  if (fence < 0) return text;
  const end = text.indexOf("```", fence + 6);
  if (end < 0) return text.slice(0, fence).trim();
  const rest = `${text.slice(0, fence)}${text.slice(end + 3)}`.trim();
  return rest || "Structured output rendered in the presentation timeline.";
}

// Prompt metadata is model-visible context, not user-facing answer prose.
// Keep it in the append-only transcript for auditability, but remove only
// exact generated metadata lines from the ordinary conversation surface.
function cleanAssistantText(text: string): string {
  return text
    .split("\n")
    .filter((line) => !/^\s*Surface:\s+(?:desktop app|web client)\.?\s*$/i.test(line))
    .filter((line) => !/^\s*primary deliverable\s*:\s*(?:produced|completed)\s*$/i.test(line))
    .filter((line) => !/^\s*completed\s*$/i.test(line))
    .join("\n")
    .replace(/^\s*\n|\n\s*$/g, "")
    .trim();
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
    return list.filter((it, i) => {
      if (it.kind === "user" || it.kind === "system") return true;
      if (it.kind === "approval" && !it.resolved) return true;
      if (it.kind === "assistant" && it.text) return true;
      if (it.kind === "tool" && !it.done && i === list.length - 1) return true;
      return false;
    });
  }
  return list;
}

function hasSettledOutcome(id: string | null): boolean {
  return !!presentationOf(id)?.items.some(
    (item) => item.kind === "outcome" && item.status !== "running" && item.content.type === "document",
  );
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

function MessageActions(props: { text: string; role: "user" | "assistant" }) {
  const [copied, setCopied] = createSignal(false);
  const [copyFailed, setCopyFailed] = createSignal(false);
  const copy = async () => {
    setCopyFailed(false);
    try {
      await navigator.clipboard.writeText(props.text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      setCopyFailed(true);
    }
  };
  const editPrompt = () => {
    window.dispatchEvent(new CustomEvent("vak:edit-prompt", { detail: { text: props.text } }));
  };

  return (
    <div class={`msg-actions msg-actions-${props.role}`} aria-label="Message actions">
      <button
        type="button"
        class="msg-action-btn"
        title={copied() ? "Copied" : copyFailed() ? "Copy failed" : "Copy text"}
        aria-label={copied() ? "Copied" : copyFailed() ? "Copy failed" : "Copy text"}
        onClick={copy}
      >
        <Show when={copied()} fallback={<Icon name="copy" size={11} />}>
          <Icon name="check" size={11} />
        </Show>
        <span>{copied() ? "Copied" : copyFailed() ? "Copy failed" : "Copy"}</span>
      </button>
      <Show when={props.role === "user"}>
        <button
          type="button"
          class="msg-action-btn"
          title="Edit prompt in composer"
          aria-label="Edit prompt in composer"
          onClick={editPrompt}
        >
          <Icon name="code" size={11} />
          <span>Edit</span>
        </button>
      </Show>
    </div>
  );
}

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
    return (
      <div class="msg user">
        <div class="msg-bubble-wrap">
          <div class="md"><Markdown text={item.text} /></div>
          <MessageActions text={item.text} role="user" />
        </div>
      </div>
    );
  }
  if (item.kind === "assistant") {
    const displayText = cleanAssistantText(item.text);
    return (
      <div class="msg assistant">
        <Show when={displayText} fallback={<span class="caret" />}>
          <Markdown text={displayText} streaming={item.streaming} />
          <Show when={item.streaming}>
            <span class="caret" />
          </Show>
        </Show>
        <Show when={!item.streaming && displayText}>
          <MessageActions text={displayText} role="assistant" />
        </Show>
      </div>
    );
  }
  if (item.kind === "thinking") {
    return (
      <details class="thinking" open={!item.done}>
        <summary>thinking</summary>
        <pre>{item.text}</pre>
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

export default function ChatPane(props: { sessionId?: string | null }) {
  // In split view each pane renders ITS OWN session; without the prop the
  // pane follows the global focus (previous behavior, unchanged).
  const sid = () => props.sessionId ?? activeId();
  let scroller!: HTMLDivElement;
  let pinned = true;
  let scrollFrame: number | null = null;
  let smoothScrolling = false;
  let smoothTimer: number | null = null;
  const [atBottom, setAtBottom] = createSignal(true);

  const onScroll = () => {
    pinned = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
    setAtBottom(pinned);
  };
  const scrollToBottom = (force = false) => {
    if (force) {
      smoothScrolling = true;
      if (smoothTimer !== null) clearTimeout(smoothTimer);
      smoothTimer = window.setTimeout(() => { smoothScrolling = false; }, 400);
      scroller.scrollTo({ top: scroller.scrollHeight, behavior: "smooth" });
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
    queueMicrotask(() => scheduleScroll(true));
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
    // Projection changes alter transcript height even when item data does
    // not. Reconcile the pinned state on the next paint.
    void density();
    void presentationOf(sid());
    scheduleScroll();
  });
  onCleanup(() => {
    if (scrollFrame !== null) cancelAnimationFrame(scrollFrame);
    if (smoothTimer !== null) clearTimeout(smoothTimer);
  });


  return (
    <div class="chat-shell">
      <Show when={sid()}>{(id) => <RunControls sessionId={id()} />}</Show>
      <div class="chat" ref={scroller} onScroll={onScroll}>
        <Show when={sid()} fallback={<EmptyChat hasSession={false} />}>
          <Show when={hydratingId() !== sid()} fallback={<TranscriptSkeleton />}>
            {/* Keep a live projection visible as soon as the runtime has
                emitted one.  The transcript remains mounted underneath so
                streaming text, approvals, and tool progress never vanish
                while the structured card is being reconciled. */}
            {/* Everyday is the reader surface: provisional projections can
                contain empty answer shells and tool diagnostics before the
                final semantic result arrives. Keep that activity in Advanced. */}
            <Show when={presentationMode() === "advanced" && isRunning(sid()) && (presentationOf(sid())?.items.length ?? 0) > 0}>
              <div class="presentation-live" aria-live="polite" aria-label="Live result preview">
                <div class="presentation-live-label"><span class="dot run" /> Live result</div>
                <PresentationTimelineView timeline={presentationOf(sid())!} sessionId={sid()!} />
              </div>
            </Show>
            {/* Keep the transcript mounted for the whole live turn. The
                presentation projection is a settled view; switching to it
                while the stream is still committing causes the visible
                conversation to blink out and reappear in a different shape. */}
            <Show when={!isRunning(sid()) && hasSettledOutcome(sid())} fallback={
              <Show when={visibleItems(itemsOf(sid())).length || awaitingNextOutput(sid())} fallback={<EmptyChat hasSession={true} />}>
                <For each={visibleItems(itemsOf(sid()))}>
                  {(it) => <ItemView item={it} sessionId={sid()} />}
                </For>
                <Show when={awaitingNextOutput(sid())}><ThinkingIndicator /></Show>
              </Show>
            }>
              <PresentationTimelineView timeline={presentationOf(sid())!} sessionId={sid()!} />
            </Show>
          </Show>
        </Show>
      </div>
      <Show when={!atBottom()}>
        <button type="button" class="scroll-latest" onClick={() => scrollToBottom(true)}>
          <Icon name="chevron" size={13} /> Latest
        </button>
      </Show>
    </div>
  );
}
