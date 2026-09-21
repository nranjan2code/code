import { createEffect, createMemo, createSignal, For, Index, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { activeId, density, itemExpanded, itemsOf, hydratingId, isRunning, presentationOf, uiPreferences, openWorkbenchExecution, openCandidateReview, workbenchExecutions, setNotice, toggleItemExpanded, sessions, agentForSession, isPreviewableArtifact, openArtifactPathInCanvas, type Item } from "../store";
import { activate, approve, isApprovalPending, openFileSmart } from "../App";
import Icon from "./Icon";
import AgentMark from "./AgentMark";
import MarkdownView from "./MarkdownView";
import MessageActions from "./MessageActions";
import PresentationTimelineView, { StructuredView } from "./PresentationRenderer";
import { serverTurnFor } from "../turnPairing";
import * as api from "../api";
import "../focusTrap";
import { assistantParts, cleanAssistantText, groupAssistantParts, isFleetingNarration, parseVakFence, stripControlScaffolding } from "../structured";
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
  const ongoing = createMemo(() => sessions().filter((session) => session.running).slice(0, 3));
  const completed = createMemo(() => sessions().filter((session) => !session.running).slice(0, 3));
  const [previews, setPreviews] = createSignal<Record<string, string>>({});
  const [previewLoaded, setPreviewLoaded] = createSignal<ReadonlySet<string>>(new Set());
  createEffect(() => {
    const targets = completed();
    for (const session of targets) {
      if (previewLoaded().has(session.session_id)) continue;
      setPreviewLoaded((current) => new Set([...current, session.session_id]));
      void api.presentation(session.session_id).then((timeline) => {
        const result = [...timeline.items].reverse().find((item) =>
          (item.role === "assistant" || item.role === "worker") &&
          (item.status === "succeeded" || item.status === "partial") &&
          item.fallback_text.trim().length > 0,
        );
        const text = result?.fallback_text.trim();
        if (text) setPreviews((current) => ({ ...current, [session.session_id]: text.slice(0, 140) }));
      }).catch(() => { /* a preview is optional; the session remains reopenable */ });
    }
  });
  return (
    <div class="chat-empty">
      <div class="chat-empty-mark">
        <span class="vak-companion" aria-hidden="true">◌</span>
        <Icon name="chat" size={16} />
      </div>
      <h2 class="chat-empty-headline">
        {props.hasSession ? "Nothing here yet" : "What would you like to do?"}
      </h2>
      <p class="chat-empty-hint">
        {props.hasSession
          ? "This conversation is ready when you are. Ask a follow-up or open its details to inspect prior work."
          : "Ask a question or hand over something to plan, find, create, remember, schedule, or complete."}
      </p>
      <Show when={!props.hasSession}>
        <Show when={ongoing().length > 0}>
          <div class="home-ongoing" aria-label="Ongoing work" aria-live="polite">
            <div class="home-ongoing-heading"><span>Ongoing</span><small>Vak is working in the background</small></div>
            <For each={ongoing()}>{(session) => <button type="button" class="home-ongoing-item" onClick={() => void activate(session.session_id)}><span class="dot run" /><span>{session.title || "Untitled task"}</span><small>Working</small></button>}</For>
          </div>
        </Show>
        <Show when={completed().length > 0}>
          <div class="home-recent" aria-label="Recent results">
            <div class="home-ongoing-heading"><span>Recent results</span><small>Pick up where you left off</small></div>
            <For each={completed()}>{(session) => <button type="button" class="home-result-row" onClick={() => void activate(session.session_id)}><span class="home-result-mark">✓</span><span><strong>{session.title || "Untitled conversation"}</strong><small>{previews()[session.session_id] || "Open this conversation to see the result."}</small></span><em>Open</em></button>}</For>
          </div>
        </Show>
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

/** One calm live state for the outcome-first conversation. Raw thinking,
 * tool calls, stdout and telemetry stay in Details/Workbench. Approvals and
 * failures remain inline because they require a decision. */
function activeWorkingState(id: string | null): { executionId?: string } | null {
  if (!isRunning(id)) return null;
  const list = itemsOf(id);
  const last = list[list.length - 1];
  if (last?.kind === "assistant" && last.streaming) return null;
  if (last?.kind === "approval" && !last.resolved) return null;
  const execution = [...list].reverse().find((item) => item.kind === "tool" && !item.done);
  return { executionId: execution?.kind === "tool" ? execution.id : undefined };
}

function WorkingIndicator(props: { sessionId: string | null; executionId?: string }) {
  return (
    <div class="working-state" aria-live="polite" aria-label={`${agentForSession(props.sessionId).name} is working`}>
      <AgentMark character={agentForSession(props.sessionId).character} size={24} state="working" class="working-state-mark" />
      <span class="working-state-copy"><strong>{agentForSession(props.sessionId).name}</strong><span>Working on it</span></span>
      <Show when={props.executionId}>
        <button type="button" onClick={() => openWorkbenchExecution(props.executionId)}>View activity</button>
      </Show>
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
    // Child agents and orchestration steps belong to the selected agent's
    // internal workspace. The user-facing chat shows outcomes, not machinery.
    if (it.kind === "worker") return false;
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

  // The conversation stays outcome-first. Detailed activity remains available
  // through the task-scoped Details surface instead of changing the transcript.
  const d = "outcome";
  if (d === "outcome") {
    // "Outcome" hides the working (thinking, tool-call) detail once it's
    // done — but a run in progress must still show *something* live, or
    // the pane reads as frozen for the entire stretch between the last
    // settled turn and this one's reply. A streaming assistant reply
    // (the outcome, forming) and the single most recent in-flight tool
    return cleanList.filter((it, i) => {
      if (it.kind === "user") {
        return Boolean(stripControlScaffolding(it.text).trim());
      }
      if (it.kind === "system") {
        const t = it.text.toLowerCase();
        if (
          t.includes("compacting context") ||
          t.includes("context compacted") ||
          t.includes("route fallback") ||
          t.includes("stop gate:") ||
          t.includes("retrying")
        ) {
          return false;
        }
        return true;
      }
      if (it.kind === "approval" && !it.resolved) return true;
      if (it.kind === "assistant") {
        if (it.streaming) return true;
        const scrubbed = cleanAssistantText(it.text);
        if (!scrubbed.trim()) return false;
        // In outcome density, suppress earlier assistant items in the same turn
        // ONLY if they are fleeting narration (e.g. "I'll search for that...").
        // Substantive answers, analyses, and reports must NEVER be hidden.
        if (isFleetingNarration(it.text)) {
          for (let j = i + 1; j < cleanList.length; j++) {
            const next = cleanList[j];
            if (next.kind === "user") break;
            if (
              next.kind === "assistant" &&
              (next.streaming || Boolean(cleanAssistantText(next.text).trim()))
            ) {
              return false;
            }
          }
        }
        return true;
      }
      // In outcome density, intermediate tool executions remain in Workbench
      // and task details rather than cluttering the chat canvas.
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
      <Show when={revision() > 0}>
        <button type="button" class="run-control" disabled={busy()} onClick={() => setChanging(!changing())}>Change plan</button>
        <span class="run-revision" title="Active outcome plan revision">Plan v{revision()}</span>
      </Show>
      <Show when={paused()}><span class="run-paused" role="status">Paused at safe boundary</span></Show>
    </div>
    <Show when={controlError()}>
      <div class="inline-error" role="alert">{controlError()} Refreshing will retry; the server remains authoritative.</div>
    </Show>
    <Show when={changing() && revision() > 0}>
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
  } as Record<string, string>)[props.item.name] ?? (() => {
    // Unmapped/MCP-namespaced tool names (e.g. "mcp__browser__navigate")
    // shouldn't leak their internal identifier verbatim — take the last
    // segment and present it in plain title case.
    const parts = props.item.name.split("__").filter(Boolean);
    const last = parts[parts.length - 1] || props.item.name;
    return last.replaceAll("_", " ").replace(/\b\w/g, (c) => c.toUpperCase());
  })();
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
            <Show when={isPreviewableArtifact(path())}>
              <button
                type="button"
                class="tool-open"
                onClick={() => openArtifactPathInCanvas(path())}
                title={`Open ${path()} in Artifact Canvas`}
              >
                <Icon name="preview" size={12} /> Open Canvas
              </button>
            </Show>
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
  const [invitees, setInvitees] = createSignal<api.CoworkingInvitation[]>([]);
  const [delegateTo, setDelegateTo] = createSignal("");
  const [delegatedName, setDelegatedName] = createSignal("");
  const [delegateError, setDelegateError] = createSignal("");
  const [delegating, setDelegating] = createSignal(false);
  createEffect(() => {
    const sessionId = props.sessionId;
    if (!sessionId || props.item.resolved) return;
    void api.listCoworkingInvitations(sessionId).then((result) => {
      setInvitees(result.invitations.filter((invitation) => invitation.status === "active"));
    }).catch(() => setInvitees([]));
  });
  const delegate = async () => {
    const sessionId = props.sessionId;
    if (!sessionId || !delegateTo() || delegating()) return;
    setDelegating(true);
    setDelegateError("");
    try {
      const result = await api.delegateCoworkingApproval(sessionId, props.item.id, delegateTo());
      setDelegatedName(result.delegated_to);
    } catch (error) {
      setDelegateError(error instanceof Error ? error.message : String(error));
    } finally { setDelegating(false); }
  };
  // Webfetch-style tools name their target under different keys; whatever
  // the tool calls its subject (url/path/command…) is what the user needs
  // to see before deciding, so it gets the prominent slot.
  const primary = createMemo<{ key: string; value: string } | null>(() => {
    try {
      const parsed = JSON.parse(props.item.argsJson) as Record<string, unknown>;
      for (const key of APPROVAL_PRIMARY_KEYS.filter((candidate) => candidate !== "command")) {
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
        .filter(([key, value]) => key !== "command" && typeof value === "string" && value.length < 180)
        .slice(0, 2)
        .map(([key, value]) => `${key.replaceAll("_", " ")}: ${String(value)}`)
        .join(" · ");
    } catch {
      return "Review the requested operation before allowing it.";
    }
  });
  const argsPretty = createMemo(() => {
    try { return JSON.stringify(JSON.parse(props.item.argsJson), null, 2); } catch { return props.item.argsJson; }
  });
  return (
    <Show when={!props.item.resolved}>
    <div class="approval" data-approval={props.item.id} role={props.item.resolved ? "status" : "alert"} aria-live={props.item.resolved ? "polite" : "assertive"} aria-label={`${props.item.resolved ? "Approval resolved" : "Approval requested"} for ${props.item.tool}`}>
    <div class="ap-head">Vak wants to use {props.item.tool}</div>
    <Show when={primary()}>
      {(p) => (
        <code class="ap-primary" title={p().value}>
          <span class="ap-primary-key">{p().key}</span>
          {p().value}
        </code>
      )}
    </Show>
    <div class="ap-reason">This needs your approval before it can continue.</div>
    <div class="ap-summary">{summary()}</div>
    <details class="ap-details">
      <summary>View request details</summary>
      <pre class="ap-args">{argsPretty()}</pre>
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
      <Show when={invitees().length > 0}>
        <div class="ap-delegation">
          <label for={`delegate-${props.item.id}`}>Ask someone in this conversation</label>
          <select id={`delegate-${props.item.id}`} value={delegateTo()} onChange={(event) => setDelegateTo(event.currentTarget.value)}>
            <option value="">Choose a person</option>
            <For each={invitees()}>{(invitation) => <option value={invitation.grant_id}>{invitation.display_name}</option>}</For>
          </select>
          <button type="button" class="btn" disabled={!delegateTo() || delegating()} onClick={() => void delegate()}>{delegating() ? "Asking…" : "Ask to decide this request"}</button>
          <Show when={delegatedName()}><span role="status">Waiting for {delegatedName()} to decide.</span></Show>
          <Show when={delegateError()}><span role="alert">{delegateError()}</span></Show>
        </div>
      </Show>
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
    </Show>
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
            <span class="turn-author-chip">{item.authorName || "You"}</span>
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
  if (item.kind === "worker") {
    return (
      <details class="worker">
        <summary>
          worker · {item.label}
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
    const groups = createMemo(() => groupAssistantParts(parts()));
    const displayText = () => parts().filter((part) => part.type === "text").map((part) => part.text).join("\n\n");

    const turnDeliverables = createMemo(() => {
      if (props.item.streaming) return [];
      const sid = props.sessionId ?? activeId();
      const allItems = sid ? itemsOf(sid) : [];
      const myIdx = allItems.findIndex((it) => it.kind === "assistant" && (it as any).key === props.item.key);

      // Gather all tool IDs belonging to this turn (from previous user turn to this assistant turn)
      const turnToolIds = new Set<string>();
      if (myIdx > 0) {
        for (let i = myIdx - 1; i >= 0; i--) {
          const prev = allItems[i];
          if (prev.kind === "user") break;
          if (prev.kind === "tool" && prev.id) {
            turnToolIds.add(prev.id);
          }
        }
      }

      const results: Array<{ name: string; path: string; execId?: string }> = [];
      const seenPaths = new Set<string>();
      const executions = workbenchExecutions();

      // Only attach artifacts through the brokered tool-call/execution id.
      // Prose and basename inference used to make a mentioned file look like
      // a produced result, and the session-wide fallback could attach an old
      // execution to every later answer.
      for (const exec of executions) {
        if (turnToolIds.has(exec.id)) {
          for (const art of exec.artifacts) {
            if (isPreviewableArtifact(art.path) && !seenPaths.has(art.path)) {
              seenPaths.add(art.path);
              results.push({
                name: art.path.split("/").pop() || art.path,
                path: art.path,
                execId: exec.id,
              });
            }
          }
        }
      }

      return results;
    });

    return (
      <div class="msg assistant">
        <div class="assistant-turn-head">
          <AgentMark character={agentForSession(props.sessionId ?? activeId()).character} size={26} state={props.item.streaming ? "working" : "idle"} class="assistant-avatar-mark" />
          <span class="assistant-name">{agentForSession(props.sessionId ?? activeId()).name}</span>
          <Show when={props.item.streaming}>
            <span class="assistant-live-pulse" title="Generating">
              <span class="dot run" />
            </span>
          </Show>
        </div>
        <div class="assistant-turn-body">
          <For each={groups()}>
            {(group) =>
              group.type === "text" ? (
                <Markdown text={group.text} streaming={props.item.streaming} />
              ) : group.cards.length === 1 ? (
                <StructuredView output={group.cards[0].output} fallback={group.cards[0].source} sessionId={props.sessionId ?? undefined} />
              ) : (
                <div class="card-group" style={{ "--card-group-count": group.cards.length }}>
                  <For each={group.cards}>
                    {(card) => (
                      <div class="card-group-item">
                        <StructuredView output={card.output} fallback={card.source} sessionId={props.sessionId ?? undefined} />
                      </div>
                    )}
                  </For>
                </div>
              )
            }
          </For>
          <Show when={!props.item.streaming && turnDeliverables().length > 0}>
            <div class="turn-artifacts-container">
              <For each={turnDeliverables()}>
                {(art, index) => (
                  <div class="turn-artifact-chip">
                    <span class="artifact-chip-icon"><Icon name="preview" size={14} /></span>
                    <div class="artifact-chip-details">
                      <span class="artifact-chip-name">{art.name}</span>
                      <span class="artifact-chip-path">{art.path}</span>
                    </div>
                    <button
                      type="button"
                      class="artifact-chip-btn"
                      onClick={() => openArtifactPathInCanvas(art.path, undefined, { sessionId: props.sessionId ?? undefined, executionId: art.execId })}
                      title={`Open ${art.path} in Artifact Canvas`}
                    >
                      <Icon name="preview" size={12} /> Open Canvas
                    </button>
                    <Show when={index() === 0 && art.execId}>
                      <button type="button" class="artifact-chip-btn" onClick={() => openCandidateReview(art.execId!, props.sessionId ?? undefined)}><Icon name="diff" size={12} /> Review draft</button>
                    </Show>
                  </div>
                )}
              </For>
            </div>
          </Show>
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
  const [shownTurns, setShownTurns] = createSignal(40);
  let loadingEarlier = false;
  let observedTurnCount = 0;
  const turns = createMemo(() => {
    const grouped: Item[][] = [[]];
    for (const item of itemsOf(sid())) {
      if (item.kind === "user") {
        if (!stripControlScaffolding(item.text).trim()) continue;
        grouped.push([]);
      }
      grouped[grouped.length - 1].push(item);
    }
    return grouped;
  });
  const displayedTurns = createMemo(() => {
    const all = turns();
    const start = Math.max(0, all.length - shownTurns());
    return all.slice(start).map((turn, offset) => ({ turn, index: start + offset }));
  });
  const working = createMemo(() => activeWorkingState(sid()));

  const onScroll = () => {
    if (smoothScrolling) return;
    pinned = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
    setAtBottom(pinned);
    if (scroller.scrollTop < 160 && shownTurns() < turns().length && !loadingEarlier) {
      loadingEarlier = true;
      const height = scroller.scrollHeight;
      const top = scroller.scrollTop;
      setShownTurns((count) => Math.min(turns().length, count + 40));
      requestAnimationFrame(() => {
        scroller.scrollTop = top + scroller.scrollHeight - height;
        loadingEarlier = false;
      });
    }
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
    setShownTurns(40);
    observedTurnCount = 0;
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
  createEffect(() => {
    const count = turns().length;
    if (observedTurnCount > 0 && count > observedTurnCount && !pinned) {
      setShownTurns((shown) => shown + count - observedTurnCount);
    }
    observedTurnCount = count;
  });
  const projectedTurn = (index: number) => {
    const timeline = presentationOf(sid());
    if (!timeline) return null;
    const serverTurn = serverTurnFor(turns()[index]?.find((item) => item.kind === "user")?.entryId, timeline.items);
    if (!serverTurn) return null;
    const items = timeline.items.filter((item) => item.turn_id === serverTurn);
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
            <Show when={visibleItems(itemsOf(sid())).length || working()} fallback={<EmptyChat hasSession={true} />}>
              <Index each={displayedTurns()}>{(entry) =>
                <Show when={projectedTurn(entry().index)} fallback={<Index each={visibleItems(entry().turn)}>{(it) => <Show when={it().kind === "assistant"} fallback={<For each={[it()]}>{(item) => <ItemView item={item} sessionId={sid()} />}</For>}><AssistantItem item={it() as Extract<Item, { kind: "assistant" }>} sessionId={sid()} /></Show>}</Index>}>
                  {(timeline) => <PresentationTimelineView timeline={timeline()} sessionId={sid()!} />}
                </Show>
              }</Index>
              <Show when={working()}>{(state) => <WorkingIndicator sessionId={sid()} executionId={state().executionId} />}</Show>
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
