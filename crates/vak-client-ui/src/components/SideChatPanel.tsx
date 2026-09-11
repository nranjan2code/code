import { createEffect, createSignal, For, Show } from "solid-js";
import { activeId, isRunning, itemsOf, setSideOpen } from "../store";
import { stopSide, sendSideQuestion } from "../App";
import { Markdown, ToolCard, stripControlScaffolding } from "./ChatPane";
import { cleanAssistantText } from "../structured";
import Icon from "./Icon";

/** `/btw` — ask with session context; never touches the main thread. */
export default function SideChatPanel() {
  let scroller!: HTMLDivElement;
  const [text, setText] = createSignal("");
  const sid = () => activeId();
  const items = () => itemsOf(sid(), "side");
  const running = () => isRunning(sid(), "side");

  const submit = () => {
    const q = text().trim();
    if (!q || !sid()) return;
    setText("");
    void sendSideQuestion(q);
  };

  createEffect(() => {
    void items().length;
    const last = items()[items().length - 1];
    if (last?.kind === "assistant") void last.text;
    queueMicrotask(() => {
      if (scroller) scroller.scrollTop = scroller.scrollHeight;
    });
  });

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      submit();
    }
  };

  return (
    <div class="sidechat">
      <div class="sidechat-h">
        <span class="sidechat-title">side chat</span>
        <span class="hint">reads this session's context · never lands on the main thread</span>
        <Show when={running()}>
          <button class="chip sm" onClick={stopSide}>stop</button>
        </Show>
        <button class="dock-close" title="Close (⌘;)" aria-label="Close side chat" onClick={() => closePanel()}><Icon name="close" /></button>
      </div>
      <div class="sidechat-body" ref={scroller}>
        <Show
          when={items().length}
          fallback={
            <div class="sidechat-empty">
              Ask about the code, an assumption, a plan — the answer stays off
              the record.
            </div>
          }
        >
          <For each={items()}>
            {(it) => (
              <>
                {(it.kind === "user" && stripControlScaffolding(it.text) && <div class="msg user"><Markdown text={stripControlScaffolding(it.text)} /></div>) ||
                  (it.kind === "assistant" && (
                    <div class="msg assistant">
                      <Markdown text={cleanAssistantText(it.text)} streaming={it.streaming} />
                    </div>
                  )) ||
                  (it.kind === "thinking" && (
                    <details class="thinking" open={!it.done}>
                      <summary>thinking</summary>
                      <pre>{it.text}</pre>
                    </details>
                  )) ||
                  (it.kind === "tool" && <ToolCard item={it} />) ||
                  (it.kind === "system" && <div class="sysnote">{stripControlScaffolding(it.text)}</div>) ||
                  <></>}
              </>
            )}
          </For>
        </Show>
      </div>
      <div class="sidechat-input">
        <textarea
          rows={1}
          placeholder={sid() ? "Ask aside… (Enter to send)" : "Select a session first"}
          value={text()}
          onInput={(e) => setText(e.currentTarget.value)}
          onKeyDown={onKey}
        />
        <button class="btn primary" disabled={!text().trim() || !sid()} onClick={submit}>
          Ask ⏎
        </button>
      </div>
    </div>
  );
}

function closePanel() {
  setSideOpen(false);
}
