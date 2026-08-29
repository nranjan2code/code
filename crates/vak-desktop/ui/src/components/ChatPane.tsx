import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import type { JSX } from "solid-js";
import { activeId, density, itemsOf, hydratingId, isRunning, openInEditor, presentationOf, type Item } from "../store";
import { approve, openFileSmart } from "../App";
import Icon from "./Icon";
import PresentationTimelineView from "./PresentationRenderer";
import MarkdownView from "./MarkdownView";

function EmptyChat() {
  return null;
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
  const d = density();
  if (d === "outcome") {
    return list.filter(
      (it) => it.kind === "user" || (it.kind === "assistant" && !it.streaming && it.text) || it.kind === "system" || (it.kind === "approval" && !it.resolved),
    );
  }
  return list;
}

function hasSettledOutcome(id: string | null): boolean {
  return !!presentationOf(id)?.items.some(
    (item) => item.kind === "outcome" && item.status !== "running" && item.content.type === "document",
  );
}

export const ToolCard = (props: { item: Extract<Item, { kind: "tool" }> }) => {
  const [open, setOpen] = createSignal(false);
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
    bash: "Run command",
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
    if (value) return value.slice(0, density() === "audit" ? 4000 : 800);
    return props.item.done ? "No output returned." : "Waiting for a result…";
  };

  return (
    <div class="tool" classList={{ err: props.item.isError, open: open(), running: !props.item.done, done: props.item.done }}>
      <button class="tool-h" aria-expanded={open()} onClick={() => setOpen((v) => !v)}>
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
            <button class="tool-open" onClick={() => void openFileSmart(path())} title={path()}>
              <Icon name="code" size={12} /> View file
            </button>
          </div>
        )}
      </Show>
      <div class="tool-result" classList={{ err: props.item.isError }}>{result()}</div>
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
  <div class="approval">
    <div class="ap-head">Approval requested — {props.item.tool}</div>
    <Show when={primary()}>
      {(p) => (
        <code class="ap-primary" title={p().value}>
          <span class="ap-primary-key">{p().key}</span>
          {p().value.length > 160 ? `${p().value.slice(0, 160)}…` : p().value}
        </code>
      )}
    </Show>
    <Show when={props.item.reason}>
      <div class="ap-reason">{props.item.reason}</div>
    </Show>
    <div class="ap-summary">{summary()}</div>
    <details class="ap-details">
      <summary>View request details</summary>
      <pre class="ap-args">{props.item.argsJson.slice(0, 2000)}</pre>
    </details>
    <Show
      when={!props.item.resolved}
      fallback={<div class="ap-done">{props.item.resolved}</div>}
    >
      <div class="ap-actions">
        <button class="btn primary" onClick={() => void approve(props.item.id, true, props.sessionId)}>
          Allow once
        </button>
        <button class="btn danger" onClick={() => void approve(props.item.id, false, props.sessionId)}>
          Deny
        </button>
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
    return <div class="msg user"><Markdown text={item.text} /></div>;
  }
  if (item.kind === "assistant") {
    return (
      <div class="msg assistant">
        <Show when={item.text} fallback={<span class="caret" />}>
          <Markdown text={item.text} streaming={item.streaming} />
          <Show when={item.streaming}>
            <span class="caret" />
          </Show>
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
  const [atBottom, setAtBottom] = createSignal(true);

  const onScroll = () => {
    pinned = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
    setAtBottom(pinned);
  };
  const scrollToBottom = (force = false) => {
    if (pinned || force) {
      scroller.scrollTo({ top: scroller.scrollHeight, behavior: force ? "smooth" : "auto" });
      pinned = true;
      setAtBottom(true);
    }
  };

  createEffect(() => {
    const id = sid();
    void id;
    pinned = true;
    setAtBottom(true);
    queueMicrotask(() => scrollToBottom(true));
  });

  createEffect(() => {
    itemsOf(sid()).length;
    queueMicrotask(scrollToBottom);
  });

  // follow streaming text growth too
  createEffect(() => {
    const list = itemsOf(sid());
    const last = list[list.length - 1];
    if (last?.kind === "assistant") void last.text;
    queueMicrotask(scrollToBottom);
  });

  return (
    <div class="chat-shell">
      <div class="chat" ref={scroller} onScroll={onScroll}>
        <Show when={sid()} fallback={<EmptyChat />}>
          <Show when={hydratingId() !== sid()} fallback={<TranscriptSkeleton />}>
            <Show when={!isRunning(sid()) && hasSettledOutcome(sid())} fallback={
              <Show when={visibleItems(itemsOf(sid())).length} fallback={<EmptyChat />}>
                <For each={visibleItems(itemsOf(sid()))}>
                  {(it) => <ItemView item={it} sessionId={sid()} />}
                </For>
              </Show>
            }>
              <PresentationTimelineView timeline={presentationOf(sid())!} sessionId={sid()!} />
            </Show>
          </Show>
        </Show>
      </div>
      <Show when={!atBottom()}>
        <button class="scroll-latest" onClick={() => scrollToBottom(true)}>
          <Icon name="chevron" size={13} /> Latest
        </button>
      </Show>
    </div>
  );
}
