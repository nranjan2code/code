import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import type { JSX } from "solid-js";
import { density, itemsOf, activeId, hydratingId, openInEditor, uiPreferences, type Item } from "../store";
import { approve, sendPrompt, newSession } from "../App";
import { renderMarkdown } from "../md";
import Icon from "./Icon";

const starters = [
  { eyebrow: "Understand", prompt: "Map this codebase and explain the architecture, key flows, and highest-risk areas." },
  { eyebrow: "Improve", prompt: "Review this project deeply and implement the highest-impact quality improvement." },
  { eyebrow: "Ship", prompt: "Find the most important unfinished feature, implement it, and verify it end to end." },
];

function EmptyChat() {
  const start = async (prompt: string) => {
    if (!activeId()) await newSession();
    void sendPrompt(prompt);
  };
  return (
    <div class="chat-empty">
      <div class="chat-empty-mark"><Icon name="spark" size={24} /></div>
      <h2>What should we build?</h2>
      <p>Describe an outcome. vakcoder will inspect the project, make the changes, and verify the result.</p>
      <Show when={uiPreferences.suggestions}><div class="starter-grid">
        <For each={starters}>
          {(starter) => (
            <button class="prompt-chip" onClick={() => void start(starter.prompt)}>
              <span>{starter.eyebrow}</span>
              <strong>{starter.prompt}</strong>
              <Icon name="chevron" size={14} />
            </button>
          )}
        </For>
      </div></Show>
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
  const d = density();
  if (d === "summary") {
    return list.filter(
      (it) => it.kind === "user" || (it.kind === "assistant" && !it.streaming && it.text) || it.kind === "system" || (it.kind === "approval" && !it.resolved),
    );
  }
  return list;
}

export const ToolCard = (props: { item: Extract<Item, { kind: "tool" }> }) => {
  const [open, setOpen] = createSignal(false);
  const argsPretty = createMemo(() => {
    try {
      return JSON.stringify(JSON.parse(props.item.argsJson), null, 2);
    } catch {
      return props.item.argsJson;
    }
  });
  return (
    <div class="tool" classList={{ err: props.item.isError, open: open() }}>
      <button class="tool-h" aria-expanded={open()} onClick={() => setOpen((v) => !v)}>
        <span class="tool-dot" />
        <span class="tool-name">{props.item.name}</span>
        <Show when={!props.item.done}>
          <span class="tool-run">running…</span>
        </Show>
        <Show when={props.item.isError}>
          <span class="tool-err">error</span>
        </Show>
        <span class="tool-chev"><Icon name="chevron" size={14} /></span>
      </button>
      <Show when={open() || density() === "verbose"}>
        <pre class="tool-args">{argsPretty()}</pre>
      </Show>
      <Show when={(props.item.preview || density() === "verbose") && props.item.done}>
        <pre class="tool-prev" classList={{ err: props.item.isError }}>
          {(props.item.preview ?? "").slice(0, density() === "verbose" ? 4000 : 800)}
        </pre>
      </Show>
    </div>
  );
};

const ApprovalCard = (props: { item: Extract<Item, { kind: "approval" }> }) => (
  <div class="approval">
    <div class="ap-head">Approval requested — {props.item.tool}</div>
    <Show when={props.item.reason}>
      <div class="ap-reason">{props.item.reason}</div>
    </Show>
    <pre class="ap-args">{props.item.argsJson.slice(0, 2000)}</pre>
    <Show
      when={!props.item.resolved}
      fallback={<div class="ap-done">{props.item.resolved}</div>}
    >
      <div class="ap-actions">
        <button class="btn primary" onClick={() => void approve(props.item.id, true)}>
          Allow once
        </button>
        <button class="btn danger" onClick={() => void approve(props.item.id, false)}>
          Deny
        </button>
      </div>
    </Show>
  </div>
);

export const Markdown = (props: { text: string; streaming?: boolean }): JSX.Element => {
  let el!: HTMLDivElement;
  createEffect(() => {
    el.innerHTML = renderMarkdown(props.text);
    if (props.streaming) el.classList.add("streaming");
    else el.classList.remove("streaming");
  });
  // delegate copy buttons + file-path links
  const onClick = (e: MouseEvent) => {
    const t = e.target as HTMLElement;
    if (t.classList.contains("cb-copy")) {
      void navigator.clipboard.writeText(t.getAttribute("data-copy") ?? "");
      t.textContent = "copied";
      setTimeout(() => (t.textContent = "copy"), 900);
      return;
    }
    const code = t.closest("code.ic[data-path]");
    if (code) {
      const pathText = code.textContent ?? "";
      if (/^[\w@.-]+(\/[\w@.-]+)+$|^\.[\w/-]+$/.test(pathText)) openInEditor(pathText);
    }
  };
  return <div class="md" ref={el} onClick={onClick} />;
};

export default function ChatPane() {
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
    const id = activeId();
    void id;
    pinned = true;
    setAtBottom(true);
    queueMicrotask(() => scrollToBottom(true));
  });

  createEffect(() => {
    itemsOf(activeId()).length;
    queueMicrotask(scrollToBottom);
  });

  // follow streaming text growth too
  createEffect(() => {
    const list = itemsOf(activeId());
    const last = list[list.length - 1];
    if (last?.kind === "assistant") void last.text;
    queueMicrotask(scrollToBottom);
  });

  return (
    <div class="chat-shell">
      <div class="chat" ref={scroller} onScroll={onScroll}>
        <Show when={activeId()} fallback={<EmptyChat />}>
          <Show when={hydratingId() !== activeId()} fallback={<TranscriptSkeleton />}>
            <Show when={visibleItems(itemsOf(activeId())).length} fallback={<EmptyChat />}>
              <For each={visibleItems(itemsOf(activeId()))}>
                {(it) => <>
              {(it.kind === "user" && <div class="msg user"><Markdown text={it.text} /></div>) ||
                (it.kind === "assistant" && (
                  <div class="msg assistant">
                    <Show when={it.text} fallback={<span class="caret" />}>
                      <Markdown text={it.text} streaming={it.streaming} />
                      <Show when={it.streaming}>
                        <span class="caret" />
                      </Show>
                    </Show>
                  </div>
                )) ||
                (it.kind === "thinking" && (
                  <details class="thinking" open={!it.done}>
                    <summary>thinking</summary>
                    <pre>{it.text}</pre>
                  </details>
                )) ||
                (it.kind === "tool" && <ToolCard item={it} />) ||
                (it.kind === "approval" && <ApprovalCard item={it} />) ||
                (it.kind === "subagent" && (
                  <details class="subagent">
                    <summary>
                      subagent · {it.label}
                      {it.isError ? " ✗" : ""}
                    </summary>
                    <pre>{it.lines.join("\n")}</pre>
                  </details>
                )) ||
                (it.kind === "system" && <div class="sysnote">{it.text}</div>)}
                </>}
              </For>
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
