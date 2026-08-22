import { createEffect, createSignal, For, Show } from "solid-js";
import { activeId, isRunning, itemsOf } from "../store";
import { sendPrompt, stopRun } from "../App";
import * as api from "../api";

interface Mention {
  start: number; // index of '@'
  query: string;
}

function detectMention(text: string, caret: number): Mention | null {
  const upto = text.slice(0, caret);
  const m = /(^|[\s(])@([\w./-]*)$/.exec(upto);
  if (!m) return null;
  return { start: caret - m[2].length - 1, query: m[2] };
}

export default function Composer(props: { cwd: string }) {
  const [text, setText] = createSignal("");
  const [mention, setMention] = createSignal<Mention | null>(null);
  const [candidates, setCandidates] = createSignal<string[]>([]);
  const [picked, setPicked] = createSignal(0);
  const [files, setFiles] = createSignal<string[]>([]);
  let ta!: HTMLTextAreaElement;

  // project file cache for @mentions (refresh when cwd changes)
  createEffect(() => {
    void props.cwd;
    api
      .fsTree(600)
      .then((r) => setFiles(r.files))
      .catch(() => setFiles([]));
  });

  const matches = () => {
    const mn = mention();
    if (!mn) return [];
    const q = mn.query.toLowerCase();
    return files()
      .filter((f) => f.toLowerCase().includes(q))
      .slice(0, 8);
  };

  const refreshMention = () => {
    const mn = detectMention(text(), ta.selectionStart);
    if (!mn) {
      setMention(null);
      return;
    }
    setMention(mn);
    setPicked(0);
    setCandidates([]);
  };

  // keep candidate list reactive with the query
  createEffect(() => {
    const mn = mention();
    if (!mn) {
      setCandidates([]);
      return;
    }
    const q = mn.query.toLowerCase();
    setCandidates(files().filter((f) => f.toLowerCase().includes(q)).slice(0, 8));
  });

  const applyPick = (path: string) => {
    const mn = mention();
    if (!mn) return;
    const t = text();
    const next = `${t.slice(0, mn.start)}@${path} ${t.slice(ta.selectionStart)}`;
    setText(next);
    setMention(null);
    queueMicrotask(() => {
      const pos = mn.start + path.length + 2;
      ta.focus();
      ta.setSelectionRange(pos, pos);
      grow();
    });
  };

  const grow = () => {
    ta.style.height = "auto";
    ta.style.height = `${Math.min(ta.scrollHeight, 220)}px`;
  };

  const submit = () => {
    const t = text().trim();
    if (!t || !activeId()) return;
    setText("");
    setMention(null);
    queueMicrotask(grow);
    void sendPrompt(t);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (mention() && candidates().length) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setPicked((p) => Math.min(p + 1, candidates().length - 1));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setPicked((p) => Math.max(p - 1, 0));
        return;
      }
      if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey)) {
        e.preventDefault();
        applyPick(candidates()[picked()]);
        return;
      }
      if (e.key === "Escape") {
        setMention(null);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      submit();
    }
    // Escape without a menu falls through to the global stop handler.
  };

  return (
    <div class="composer">
      <Show when={mention() && candidates().length}>
        <div class="mention-menu">
          <For each={candidates()}>
            {(f, i) => (
              <button
                class="mention-item"
                classList={{ on: picked() === i() }}
                onMouseEnter={() => setPicked(i())}
                onClick={() => applyPick(f)}
              >
                {f}
              </button>
            )}
          </For>
        </div>
      </Show>
      <div class="composer-cwd" title={props.cwd}>
        {props.cwd.split("/").pop()}
      </div>
      <textarea
        ref={ta}
        rows={1}
        placeholder={
          activeId()
            ? isRunning(activeId())
              ? "Steer while running — Enter queues a correction…"
              : "Describe a task… type @ to attach file context"
            : "Start or select a session…"
        }
        value={text()}
        onInput={(e) => {
          setText(e.currentTarget.value);
          refreshMention();
          grow();
        }}
        onKeyUp={(e) => {
          if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)) refreshMention();
        }}
        onClick={refreshMention}
        onKeyDown={onKeyDown}
      />
      <Show when={isRunning(activeId())}>
        <button class="btn danger composer-stop" title="Stop (Esc)" onClick={stopRun}>
          ■ Stop
        </button>
      </Show>
      <button class="btn primary" disabled={!activeId() || !text().trim()} onClick={submit}>
        Send ⏎
      </button>
    </div>
  );
}
