import { createEffect, createMemo, createSignal, For, Index, onCleanup, onMount, Show, untrack } from "solid-js";
import type { JSX } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { activeId, activeAgentId, backend, setAgentCreateOpen, openConnect, setTechnicalDetails, technicalDetails, itemExpanded, itemsOf, hydratingId, isRunning, lastSubmittedPrompt, presentationOf, openWorkbenchExecution, workbenchExecutions, setNotice, toggleItemExpanded, sessions, agentForSession, narrowViewport, setGreetingsShown, isPreviewableArtifact, openArtifactCanvas, openArtifactFile, type Item } from "../store";
import { activate, approve, isApprovalPending, openFileSmart } from "../App";
import Icon from "./Icon";
import Sheet from "./Sheet";
import AgentMark from "./AgentMark";
import SetupBanner from "./SetupBanner";
import MarkdownView from "./MarkdownView";
import MessageActions from "./MessageActions";
import PresentationTimelineView, { Artifact, ResultCard, StructuredView } from "./PresentationRenderer";
import { attachFiles } from "../attachFiles";
import type { OutputItem } from "../types";
import { hasSettledProjection, serverTurnFor } from "../turnPairing";
import * as api from "../api";
import "../focusTrap";
import { assistantParts, cleanAssistantText, groupAssistantParts, parseVakFence, stripControlScaffolding } from "../structured";
import Skeleton from "./Skeleton";
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
 * task with nothing rendered at the current detail level. Previously both
 * rendered `null` — a void with no headline, hint, or affordance —
 * despite DESIGN.md naming "the chat empty state" as the canonical use
 * of the headline type scale (22px/620/-0.02em) it defines.
 */
/** The starters an empty conversation offers, each with an everyday example
 * (docs/design/75 §6.2). The prompt is what lands in the message box. */
const STARTERS: readonly { label: string; example: string; prompt: string }[] = [
  { label: "Research a question", example: "Compare three laptops for a student", prompt: "Compare three laptops for a student and recommend one." },
  { label: "Write or rewrite", example: "Make this email warmer and shorter", prompt: "Make this email warmer and shorter: " },
  { label: "Analyze data", example: "What changed in this spreadsheet?", prompt: "What changed in this spreadsheet? " },
  { label: "Plan something", example: "A relaxed Saturday with the kids", prompt: "Plan a relaxed Saturday with the kids." },
  {
    label: "Plan my day",
    example: "Check today's events and important email",
    prompt: "Give me a brief plan for today using my connected calendar, recent email that may need attention, and open commitments if they are available to this Agent. Use only sources this Agent is allowed to read, cite each event or message you rely on, and say what you could not access. Keep this read-only: do not send email or change calendar events.",
  },
];

function EmptyChat(props: { hasSession: boolean }) {
  // While a greeting is on screen it carries the setup card (App.tsx).
  onMount(() => setGreetingsShown((n) => n + 1));
  onCleanup(() => setGreetingsShown((n) => n - 1));
  const ongoing = createMemo(() => sessions().filter((session) => session.running).slice(0, 3));
  // A conversation with no title has had no message yet: nothing to resume.
  const completed = createMemo(() => sessions().filter((session) => !session.running && session.title).slice(0, 3));
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
        <AgentMark character={agentForSession(activeId()).character} size={narrowViewport() ? 72 : 104} />
      </div>
      <h2 class="chat-empty-headline">Hi, I'm {agentForSession(activeId()).name}.</h2>
      <p class="chat-empty-hint">
        {props.hasSession
          ? "Pick up where you left off, or ask something new."
          : "Ask a question, plan something or hand over a task. Talk or type, whichever is easier."}
      </p>
      <SetupBanner inGreeting />
      <Show when={!props.hasSession}>
        <MeetNote />
      </Show>
      <Show when={!props.hasSession}>
        <Show when={ongoing().length > 0}>
          <div class="home-ongoing" aria-label="Ongoing work" aria-live="polite">
            <div class="home-ongoing-heading"><span>Ongoing</span><small>Vakyartha is working in the background</small></div>
            <For each={ongoing()}>{(session) => <button type="button" class="home-ongoing-item" onClick={() => void activate(session.session_id)}><span class="dot run" /><span>{session.title || "Untitled task"}</span><small>Working</small></button>}</For>
          </div>
        </Show>
        <Show when={completed().length > 0}>
          <div class="home-recent" aria-label="Recent results">
            <div class="home-ongoing-heading"><span>Recent results</span><small>Pick up where you left off</small></div>
            <For each={completed()}>{(session) => <button type="button" class="home-result-row" onClick={() => void activate(session.session_id)}><span class="home-result-mark"><Icon name="check" size={14} /></span><span><strong>{session.title || "Untitled conversation"}</strong><small>{previews()[session.session_id] || "Open this conversation to see the result."}</small></span><em>Open</em></button>}</For>
          </div>
        </Show>
      </Show>
      <div class="chat-empty-examples" aria-label="Things Vakyartha can help with">
        <For each={STARTERS}>
          {(starter) => (
            <button
              type="button"
              class="chat-empty-example"
              onClick={() => window.dispatchEvent(new CustomEvent("vak:edit-prompt", { detail: { text: starter.prompt } }))}
            >
              <strong>{starter.label}</strong>
              <span>{starter.example}</span>
            </button>
          )}
        </For>
      </div>
    </div>
  );
}

const MEET_SEEN_KEY = "vak.onboarded";

/** The one-time introduction on a fresh home: that people can make their own
 * agents, and whether to show technical details. It sits in the greeting
 * instead of covering it, and goes for good once answered. */
function MeetNote() {
  const seen = () => { try { return localStorage.getItem(MEET_SEEN_KEY) === "1"; } catch { return false; } };
  const [dismissed, setDismissed] = createSignal(seen());
  const [ownAgents, setOwnAgents] = createSignal<number | null>(null);
  createEffect(() => {
    if (dismissed() || !backend().ready || ownAgents() !== null) return;
    void api.listAgents().then((r) => setOwnAgents(r.agents.filter((agent) => agent.id !== "vak").length)).catch(() => setOwnAgents(1));
  });
  const dismiss = () => {
    try { localStorage.setItem(MEET_SEEN_KEY, "1"); } catch { /* the note still closes for this window */ }
    setDismissed(true);
  };
  return (
    <Show when={!dismissed() && ownAgents() === 0 && activeAgentId() === "vak"}>
      <section class="meet-note" aria-label="Getting started">
        <p>You can also make your own agents for the things you do often, each with its own name, character and way of working.</p>
        <label class="onboarding-technical"><input type="checkbox" checked={technicalDetails()} onChange={(event) => setTechnicalDetails(event.currentTarget.checked)} /> I build software: show technical details</label>
        <div class="meet-note-actions">
          <button type="button" class="btn primary sm" onClick={() => { dismiss(); setAgentCreateOpen(true); }}><Icon name="add" size={14} /> Create an agent</button>
          <button type="button" class="btn sm" onClick={dismiss}>Not now</button>
        </div>
      </section>
    </Show>
  );
}

/** One calm live state for the outcome-first conversation. Raw thinking,
 * tool calls, stdout and telemetry stay in Details/Workbench. Approvals and
 * failures remain inline because they require a decision. */
function activeWorkingState(id: string | null): { executionId?: string } | null {
  if (!isRunning(id)) return null;
  const list = itemsOf(id);
  const last = list[list.length - 1];
  if ((last?.kind === "approval" || last?.kind === "question") && !last.resolved) return null;
  const execution = [...list].reverse().find((item) => item.kind === "tool" && !item.done);
  return { executionId: execution?.kind === "tool" ? execution.id : undefined };
}


function TranscriptSkeleton() {
  return (
    <Skeleton kind="transcript" class="transcript-skeleton" label="Loading the conversation" />
  );
}

function visibleItems(list: Item[], liveTurn = false): Item[] {
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
      // Model text in the active turn can be a draft that is replaced by a
      // tool-backed result. Keep the conversation calm until the turn settles.
      if (liveTurn || it.streaming) return false;
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
    // The working indicator is the live state; only settled answer content
    // appears in the everyday conversation.
    return cleanList.filter((it, i) => {
      if (it.kind === "user") {
        return Boolean(stripControlScaffolding(it.text).trim());
      }
      // Runtime bookkeeping (retries, route fallback, context compaction,
      // stop-hook continuations) never reaches the client at all now — the
      // server-side projection (vak-server/src/client_events.rs) drops it
      // before it becomes a `ClientEvent`, so every "system" item here is
      // already something meant for the reader.
      if (it.kind === "system") return true;
      if ((it.kind === "approval" || it.kind === "question") && !it.resolved) return true;
      if (it.kind === "assistant") {
        const scrubbed = cleanAssistantText(it.text);
        if (!scrubbed.trim()) return false;
        // Before the durable projection arrives, show at most the latest
        // answer from this turn. Earlier assistant messages are drafts.
        for (let j = i + 1; j < cleanList.length; j++) {
          const next = cleanList[j];
          if (next.kind === "user") break;
          if (next.kind === "assistant" && Boolean(cleanAssistantText(next.text).trim())) {
            return false;
          }
        }
        return true;
      }
      // With technical details off, intermediate tool executions remain in Workbench
      // and task details rather than cluttering the chat canvas.
      return false;
    });
  }
  return cleanList;
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
      return shown.slice(0, technicalDetails() ? 4000 : 800);
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
                onClick={() => openArtifactFile(path())}
                title="Open in Canvas"
              >
                <Icon name="preview" size={12} /> Open
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
            title="View activity in Workbench"
          >
            <Icon name="terminal" size={12} /> Inspect in Workbench
          </button>
        </div>
      </Show>
      <Show when={open() || technicalDetails() || props.item.isError || !props.item.done}>
        <div class="tool-result" classList={{ err: props.item.isError }}>{result()}</div>
      </Show>
      <details class="tool-details" open={open() || technicalDetails()}>
        <summary>View request details</summary>
        <pre class="tool-args">{argsPretty()}</pre>
      </details>
    </div>
  );
};

/** Arg keys that name the subject of a webfetch-style tool call. */
const APPROVAL_PRIMARY_KEYS = ["url", "path", "file_path", "command", "file", "dir"] as const;

/** A worker waiting on one question. The answer is information for the worker,
 *  never permission: a gated action it then takes still asks separately. */
const QuestionCard = (props: { item: Extract<Item, { kind: "question" }>; sessionId?: string | null }) => {
  const [text, setText] = createSignal("");
  const [sending, setSending] = createSignal(false);
  const [error, setError] = createSignal("");
  const send = async (answer: string) => {
    const sessionId = props.sessionId ?? activeId();
    const value = answer.trim();
    if (!sessionId || !value || sending()) return;
    setSending(true);
    setError("");
    try {
      await api.answerQuestion(sessionId, props.item.id, value);
    } catch (failure) {
      const status = (failure as { status?: number })?.status;
      setError(status === 404
        ? "This question was already answered or has expired."
        : "Your answer could not be sent. Try again.");
    } finally {
      setSending(false);
    }
  };
  return (
    <Show when={!props.item.resolved}>
      <div class="approval question" data-question={props.item.id} role="alert" aria-live="assertive" aria-label={`${props.item.label} has a question`}>
        <div class="ap-head">{props.item.label} has a question</div>
        <div class="q-text">{props.item.question}</div>
        <Show when={props.item.options.length > 0}>
          <div class="ap-actions q-options">
            <For each={props.item.options}>
              {(option) => (
                <button type="button" class="btn" disabled={sending()} onClick={() => void send(option)}>{option}</button>
              )}
            </For>
          </div>
        </Show>
        <form class="q-form" onSubmit={(event) => { event.preventDefault(); void send(text()); }}>
          <label class="q-label" for={`question-${props.item.id}`}>Your answer</label>
          <input
            id={`question-${props.item.id}`}
            class="q-input"
            type="text"
            value={text()}
            maxLength={2000}
            disabled={sending()}
            onInput={(event) => setText(event.currentTarget.value)}
          />
          <button type="submit" class="btn primary" disabled={sending() || !text().trim()}>
            {sending() ? "Sending…" : "Send answer"}
          </button>
        </form>
        <div class="ap-reason">Your answer helps it continue. It does not approve any action.</div>
        <Show when={error()}><div class="q-error" role="alert">{error()}</div></Show>
      </div>
    </Show>
  );
};

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
    <div class="ap-head">Vakyartha wants to use {props.item.tool}</div>
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
      <Sheet
        size="narrow"
        title="Always allow this?"
        subtitle="Matching requests in this workspace will go ahead without asking."
        onClose={() => setShowRulePreview(false)}
        footer={<>
          <button type="button" class="btn" onClick={() => setShowRulePreview(false)}>Cancel</button>
          <button type="button" class="btn primary" onClick={() => { setShowRulePreview(false); void approve(props.item.id, true, props.sessionId, true); }}>Always allow</button>
        </>}
      >
        <dl class="ap-rule-preview">
          <div><dt>Applies to</dt><dd><code>{props.item.tool}</code></dd></div>
          <div><dt>Where</dt><dd>This workspace</dd></div>
          <div><dt>Undo it</dt><dd>Settings → Privacy and safety</dd></div>
        </dl>
      </Sheet>
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
 * viewer. Items are immutable snapshots replaced by identity in the store, so
 * the row is rebuilt whenever the item's identity changes. That must happen
 * here rather than in the caller: the chat renders turns through
 * position-keyed `<Index>`, where the item at a position changes when the
 * session switches or the turn window moves, and a row built once kept showing
 * the previous conversation's message beside the new one's answer.
 */
export const ItemView = (props: { item: Item; sessionId?: string | null }): JSX.Element =>
  <Show when={props.item} keyed>{(item) => itemBody(item, props.sessionId)}</Show>;

/** An attached file as the same file card a deliverable gets (U6). */
function attachedFileItem(file: api.InboxFile, sessionId?: string | null): OutputItem {
  const size = file.bytes < 1024 * 1024 ? `${Math.max(1, Math.round(file.bytes / 1024))} KB` : `${(file.bytes / (1024 * 1024)).toFixed(1)} MB`;
  return {
    id: `attached:${file.path}`,
    turn_id: "",
    timestamp: "",
    role: "user",
    kind: "artifact",
    status: "succeeded",
    content: { type: "artifact", artifact: { name: file.name, path: file.path, description: `Attached · ${size}` } },
    provenance: sessionId ? { session_id: sessionId } : undefined,
    actions: [],
    fallback_text: file.path,
  } as OutputItem;
}

/** A file a run just produced, before the run's settled result arrives with
 * its status; the first one of a run offers Review changes, as the settled
 * card does. */
function deliverableItem(art: { name: string; path: string; execId?: string }, first: boolean, sessionId?: string | null): OutputItem {
  return {
    id: `deliverable:${art.execId ?? ""}:${art.path}`,
    turn_id: "",
    timestamp: "",
    role: "tool",
    kind: "artifact",
    status: "succeeded",
    content: { type: "artifact", artifact: { name: art.name, path: art.path } },
    provenance: { session_id: sessionId ?? undefined, tool_call_id: art.execId },
    actions: first && art.execId ? [{ id: `review-${art.execId}`, label: "Review changes", verb: "review_draft", data: { execution_id: art.execId } }] : [],
    fallback_text: art.path,
  } as OutputItem;
}

function itemBody(item: Item, sessionId?: string | null): JSX.Element {
  if (item.kind === "user") {
    const text = stripControlScaffolding(item.text);
    if (!text && !item.files?.length) return null;
    return (
      <div class="msg user">
        <div class="msg-bubble-wrap">
          <Show when={item.authorName}>
            <div class="user-turn-head">
              <span class="turn-author-chip">{item.authorName}</span>
            </div>
          </Show>
          <Show when={item.files?.length}>
            <div class="msg-user-files" aria-label="Attached files">
              <For each={item.files}>{(file) => <Artifact item={attachedFileItem(file, sessionId)} />}</For>
            </div>
          </Show>
          <Show when={text}>
            <div class="msg-user-content"><Markdown text={text} /></div>
            <MessageActions text={text} role="user" />
          </Show>
        </div>
      </div>
    );
  }
  if (item.kind === "assistant") return <AssistantItem item={item} sessionId={sessionId} />;
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
    return <ApprovalCard item={item} sessionId={sessionId} />;
  }
  if (item.kind === "question") {
    return <QuestionCard item={item} sessionId={sessionId} />;
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
  if (item.needs === "ai-service") {
    return (
      <div class="sysnote needs-service" role="status">
        <span>{item.text}</span>
        <button type="button" class="btn primary sm" onClick={() => openConnect()}>Connect</button>
      </div>
    );
  }
  return <div class="sysnote">{item.text}</div>;
}

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
          <AgentMark character={agentForSession(props.sessionId ?? activeId()).character} motion={agentForSession(props.sessionId ?? activeId()).animation} size={26} state={props.item.streaming ? "working" : "idle"} class="assistant-avatar-mark" />
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
            <div class="primary-result-files">
              <For each={turnDeliverables()}>
                {(art, index) => <ResultCard item={deliverableItem(art, index() === 0, props.sessionId ?? activeId())} sessionId={props.sessionId ?? activeId() ?? ""} />}
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

// View state only: changing agents or opening a canvas must not discard the
// reader's position. Kept in memory, never written into conversation data.
const readingPositions = new Map<string, { top: number; pinned: boolean; start: number; end: number }>();

export default function ChatPane(props: { sessionId?: string | null }) {
  // In split view each pane renders ITS OWN session; without the prop the
  // pane follows the global focus (previous behavior, unchanged).
  const sid = () => props.sessionId ?? activeId();
  let scroller!: HTMLDivElement;
  let content!: HTMLDivElement;
  let pinned = true;
  let scrollFrame: number | null = null;
  let userScrollIntentUntil = 0;
  let renderedSid: string | null = null;
  let restoring = false;
  let handledSubmission = untrack(() => lastSubmittedPrompt()?.sequence ?? 0);
  const [atBottom, setAtBottom] = createSignal(true);
  const [turnWindow, setTurnWindow] = createSignal({ start: 0, end: 40 });
  const [activeTurn, setActiveTurn] = createSignal(1);
  const [hoveredTurn, setHoveredTurn] = createSignal<number | null>(null);
  let loadingWindow = false;
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
    const { start, end } = turnWindow();
    return all.slice(start, end).map((turn, offset) => ({ turn, index: start + offset }));
  });
  const navigableTurnCount = () => Math.max(0, turns().length - 1);
  const turnPreview = (index: number) => {
    const user = turns()[index]?.find((item) => item.kind === "user");
    const text = user?.kind === "user" ? stripControlScaffolding(user.text).replace(/\s+/g, " ").trim() : "";
    return text.length > 130 ? `${text.slice(0, 127)}…` : text || `Turn ${index}`;
  };
  const tickIndices = createMemo(() => {
    const count = navigableTurnCount();
    const marks = Math.min(count, 72);
    return Array.from({ length: marks }, (_, position) =>
      marks === 1 ? 1 : 1 + Math.round(position * (count - 1) / (marks - 1)));
  });
  const nearestTick = (turn: number) => tickIndices().reduce<number | null>((closest, index) =>
    closest === null || Math.abs(index - turn) < Math.abs(closest - turn) ? index : closest, null);
  const activeTick = createMemo(() => nearestTick(activeTurn()));
  const hoveredTick = createMemo(() => {
    const hovered = hoveredTurn();
    return hovered === null ? null : nearestTick(hovered);
  });
  const turnAtPointer = (event: MouseEvent | PointerEvent) => {
    const bounds = event.currentTarget instanceof HTMLElement ? event.currentTarget.getBoundingClientRect() : null;
    if (!bounds) return 1;
    const fraction = Math.max(0, Math.min(1, (event.clientY - bounds.top) / bounds.height));
    return 1 + Math.round(fraction * (navigableTurnCount() - 1));
  };
  const updateActiveTurn = () => {
    const candidates = content?.querySelectorAll<HTMLElement>("[data-turn-index]");
    if (!candidates?.length) return;
    const threshold = scroller.getBoundingClientRect().top + 40;
    let current = Number(candidates[0].dataset.turnIndex) || 1;
    for (const candidate of candidates) {
      if (candidate.getBoundingClientRect().top > threshold) break;
      current = Number(candidate.dataset.turnIndex) || current;
    }
    setActiveTurn(current);
  };
  const scrollToTurn = (index: number) => {
    const count = navigableTurnCount();
    if (!count) return;
    const target = Math.max(1, Math.min(count, index));
    pinned = false;
    setAtBottom(false);
    setActiveTurn(target);
    const window = turnWindow();
    if (target < window.start || target >= window.end) {
      setTurnWindow({ start: Math.max(0, target - 20), end: Math.min(turns().length, target + 21) });
    }
    requestAnimationFrame(() => {
      const element = content.querySelector<HTMLElement>(`[data-turn-index="${target}"]`);
      if (!element) return;
      scroller.scrollTo({ top: scroller.scrollTop + element.getBoundingClientRect().top - scroller.getBoundingClientRect().top - 22, behavior: "auto" });
      updateActiveTurn();
    });
  };
  const working = createMemo(() => activeWorkingState(sid()));
  // A turn that appears after its conversation has loaded is new, and
  // arrives with motion; the turns a conversation opens with do not. Opening
  // always loads (activate hydrates), and the conversation becomes active a
  // moment before that load starts, so it counts as loaded only once its load
  // has been seen to start and then finish.
  const [settledSid, setSettledSid] = createSignal<string | null>(null);
  let loadingSid: string | null = null;
  createEffect(() => {
    const id = sid();
    const loading = hydratingId();
    if (!id) return;
    if (loading === id) { loadingSid = id; return; }
    if (loadingSid !== id) return;
    const frame = requestAnimationFrame(() => setSettledSid(id));
    onCleanup(() => cancelAnimationFrame(frame));
  });
  // An empty conversation shows the greeting, which reads from the top.
  const showsGreeting = createMemo(() => !sid() || !(visibleItems(itemsOf(sid())).length || working()));

  const onScroll = () => {
    if (restoring) return;
    const nearBottom = turnWindow().end >= turns().length && scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 2;
    // Layout growth and browser scroll anchoring can dispatch scroll events.
    // Only an actual reader action may disengage following the live turn.
    if (nearBottom) pinned = true;
    else if (performance.now() < userScrollIntentUntil) pinned = false;
    setAtBottom(pinned);
    updateActiveTurn();
    if (renderedSid) readingPositions.set(renderedSid, { top: scroller.scrollTop, pinned, ...turnWindow() });
    if (scroller.scrollTop < 160 && turnWindow().start > 0 && !loadingWindow) {
      loadingWindow = true;
      const anchor = [...content.querySelectorAll<HTMLElement>("[data-turn-index]")]
        .find((element) => element.getBoundingClientRect().bottom > scroller.getBoundingClientRect().top);
      const anchorIndex = anchor?.dataset.turnIndex;
      const anchorTop = anchor?.getBoundingClientRect().top;
      setTurnWindow((window) => {
        const start = Math.max(0, window.start - 40);
        return { start, end: Math.min(window.end, start + 120) };
      });
      requestAnimationFrame(() => {
        const restored = content.querySelector<HTMLElement>(`[data-turn-index="${anchorIndex}"]`);
        if (restored && anchorTop !== undefined) scroller.scrollTop += restored.getBoundingClientRect().top - anchorTop;
        loadingWindow = false;
      });
    } else if (scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 160 && turnWindow().end < turns().length && !loadingWindow) {
      loadingWindow = true;
      const anchor = [...content.querySelectorAll<HTMLElement>("[data-turn-index]")]
        .find((element) => element.getBoundingClientRect().bottom > scroller.getBoundingClientRect().top);
      const anchorIndex = anchor?.dataset.turnIndex;
      const anchorTop = anchor?.getBoundingClientRect().top;
      setTurnWindow((window) => {
        const end = Math.min(turns().length, window.end + 40);
        return { start: Math.max(window.start, end - 120), end };
      });
      requestAnimationFrame(() => {
        const restored = content.querySelector<HTMLElement>(`[data-turn-index="${anchorIndex}"]`);
        if (restored && anchorTop !== undefined) scroller.scrollTop += restored.getBoundingClientRect().top - anchorTop;
        loadingWindow = false;
      });
    }
  };
  const markUserScrollIntent = () => { userScrollIntentUntil = performance.now() + 1000; };
  const scrollToBottom = (force = false) => {
    if (force) {
      const count = turns().length;
      if (turnWindow().end < count || count - turnWindow().start > 120) {
        setTurnWindow({ start: Math.max(0, count - 40), end: count });
        requestAnimationFrame(() => scrollToBottom(true));
        return;
      }
      scroller.scrollTo({ top: scroller.scrollHeight, behavior: "auto" });
      pinned = true;
      setAtBottom(true);
      setActiveTurn(Math.max(1, count - 1));
      return;
    }
    if (pinned && !showsGreeting()) {
      scroller.scrollTo({ top: scroller.scrollHeight, behavior: "auto" });
      pinned = true;
      setAtBottom(true);
    }
  };
  const scheduleScroll = (force = false) => {
    if (force) {
      if (scrollFrame !== null) cancelAnimationFrame(scrollFrame);
      scrollFrame = requestAnimationFrame(() => {
        scrollFrame = null;
        scrollToBottom(true);
      });
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
    renderedSid = id;
    const saved = id ? readingPositions.get(id) : undefined;
    pinned = saved?.pinned ?? true;
    userScrollIntentUntil = 0;
    setAtBottom(pinned);
    const count = untrack(() => turns().length);
    setTurnWindow(saved && !saved.pinned ? { start: saved.start, end: saved.end } : { start: Math.max(0, count - 40), end: count });
    setActiveTurn(Math.max(1, count - 1));
    setHoveredTurn(null);
    observedTurnCount = saved ? count : 0;
    restoring = true;
    const frame = requestAnimationFrame(() => {
      if (saved && !saved.pinned) scroller.scrollTop = saved.top;
      else scrollToBottom();
      restoring = false;
      updateActiveTurn();
    });
    onCleanup(() => cancelAnimationFrame(frame));
  });

  createEffect(() => {
    const submitted = lastSubmittedPrompt();
    if (!submitted || submitted.sequence === handledSubmission) return;
    handledSubmission = submitted.sequence;
    if (submitted.sessionId !== untrack(sid)) return;
    pinned = true;
    userScrollIntentUntil = 0;
    setAtBottom(true);
    scheduleScroll(true);
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
    // The detail switch alters transcript layout. Reconcile the pinned state on next paint.
    void technicalDetails();
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
  });
  createEffect(() => {
    const count = turns().length;
    if (count > observedTurnCount && pinned) {
      setTurnWindow((window) => ({
        start: observedTurnCount === 0 ? Math.max(0, count - 40) : count - window.start > 120 ? Math.max(0, count - 80) : window.start,
        end: count,
      }));
    }
    observedTurnCount = count;
  });
  const projectedTurn = (index: number) => {
    const timeline = presentationOf(sid());
    if (!timeline) return null;
    const serverTurn = serverTurnFor(turns()[index]?.find((item) => item.kind === "user")?.entryId, timeline.items);
    if (!serverTurn) return null;
    const items = timeline.items.filter((item) => item.turn_id === serverTurn);
    // A failed or capped run can still have a real file or card, plus its
    // failure receipt, without a final assistant message. Preserve that
    // partial work in conversation after the run settles.
    if (!hasSettledProjection(items)) return null;
    if (index === turns().length - 1 && isRunning(sid())) return null;
    return { ...timeline, items };
  };
  const [fileOver, setFileOver] = createSignal(false);
  const carriesFiles = (event: DragEvent) => Array.from(event.dataTransfer?.types ?? []).includes("Files");
  return (
    <div
      class="chat-shell"
      classList={{ "file-drop": fileOver() }}
      onDragOver={(event) => {
        if (!carriesFiles(event)) return;
        event.preventDefault();
        setFileOver(true);
      }}
      onDragLeave={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setFileOver(false);
      }}
      onDrop={(event) => {
        setFileOver(false);
        if (!carriesFiles(event) || !event.dataTransfer?.files.length) return;
        event.preventDefault();
        attachFiles(Array.from(event.dataTransfer.files));
      }}
    >
      <button type="button" class="chat-daily-canvas-link" onClick={() => openArtifactCanvas({ kind: "daily_mail_calendar", title: "Today", agentId: agentForSession(sid()).id })}>Today</button>
      <Show when={navigableTurnCount() > 1}>
        <nav class="turn-rail" aria-label="Conversation turns">
          <div
            class="turn-rail-track"
            style={{ height: `min(100%, ${tickIndices().length * 10}px)` }}
            role="slider"
            tabIndex={0}
            aria-label="Navigate conversation turns"
            aria-valuemin={1}
            aria-valuemax={navigableTurnCount()}
            aria-valuenow={activeTurn()}
            aria-valuetext={`Turn ${activeTurn()}: ${turnPreview(activeTurn())}`}
            onPointerEnter={(event) => setHoveredTurn(turnAtPointer(event))}
            onPointerMove={(event) => setHoveredTurn(turnAtPointer(event))}
            onPointerLeave={() => setHoveredTurn(null)}
            onBlur={() => setHoveredTurn(null)}
            onClick={(event) => scrollToTurn(turnAtPointer(event))}
            onKeyDown={(event) => {
              const next = event.key === "ArrowUp" ? activeTurn() - 1 : event.key === "ArrowDown" ? activeTurn() + 1 : event.key === "Home" ? 1 : event.key === "End" ? navigableTurnCount() : null;
              if (next === null) return;
              event.preventDefault();
              scrollToTurn(next);
              setHoveredTurn(next);
            }}
          >
            <For each={tickIndices()}>{(index) => <span class={`turn-rail-tick${index === activeTick() ? " active" : ""}${index === hoveredTick() ? " hovered" : ""}`} aria-hidden="true" />}</For>
            <div class={`turn-rail-preview${hoveredTurn() !== null ? " visible" : ""}`}
              aria-hidden="true"
              style={{ top: `clamp(25px, ${(Math.max(1, hoveredTurn() ?? activeTurn()) - 1) / Math.max(1, navigableTurnCount() - 1) * 100}%, calc(100% - 25px))` }}>
              <small>Turn {hoveredTurn() ?? activeTurn()} of {navigableTurnCount()}</small>
              <span>{turnPreview(hoveredTurn() ?? activeTurn())}</span>
            </div>
          </div>
        </nav>
      </Show>
      <div class="chat" ref={scroller} onScroll={onScroll} onWheel={markUserScrollIntent} onTouchStart={markUserScrollIntent} onPointerDown={markUserScrollIntent} onKeyDown={(event) => { if (["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " "].includes(event.key)) markUserScrollIntent(); }}>
        <div ref={content}>
        <Show when={sid()} fallback={<EmptyChat hasSession={false} />}>
          <Show when={hydratingId() !== sid() || itemsOf(sid()).length > 0} fallback={<TranscriptSkeleton />}>
            {/* Unified continuous chat canvas: The transcript stays permanently mounted
                across live and settled states so streaming cards, settled cards, approvals,
                and message actions maintain an unbroken, flicker-free rendering lifecycle. */}
            <Show when={visibleItems(itemsOf(sid())).length || working()} fallback={<EmptyChat hasSession={itemsOf(sid()).some((item) => item.kind === "user")} />}>
                <Index each={displayedTurns()}>{(entry) => { const arrived = untrack(() => settledSid() === sid()); return <div class={arrived ? "chat-turn arrived" : "chat-turn"} data-turn-index={entry().index}>
                <Index each={visibleItems(entry().turn).filter((item) => item.kind === "user")}>{(it) => <ItemView item={it()} sessionId={sid()} />}</Index>
                <Show when={entry().index === turns().length - 1 && working()}>
                  <div class="assistant-working-state" role="status" aria-live="polite">
                    <AgentMark character={agentForSession(sid()).character} motion={agentForSession(sid()).animation} size={28} state="working" class="assistant-working-mark" />
                    <span class="assistant-working-name">{agentForSession(sid()).name}</span>
                    <span class="assistant-working-label">Working on it</span>
                    <span class="dot run assistant-working-dot" aria-label="Working" />
                  </div>
                </Show>
                <Show when={projectedTurn(entry().index)} fallback={<Index each={visibleItems(entry().turn, entry().index === turns().length - 1 && isRunning(sid())).filter((item) => item.kind !== "user")}>{(it) => <Show when={it().kind === "assistant"} fallback={<ItemView item={it()} sessionId={sid()} />}><AssistantItem item={it() as Extract<Item, { kind: "assistant" }>} sessionId={sid()} /></Show>}</Index>}>
                  {(timeline) => <div class="turn-result"><PresentationTimelineView timeline={timeline()} sessionId={sid()!} allowContinuation={entry().index === turns().length - 1} hideUser /></div>}
                </Show>
              </div>; }}</Index>
            </Show>
          </Show>
        </Show>
        </div>
      </div>
      <Show when={!atBottom() && !showsGreeting()}>
        <button type="button" class="scroll-latest" onClick={() => scrollToBottom(true)}>
          <Icon name="chevron" size={13} /> Latest
        </button>
      </Show>
    </div>
  );
}
