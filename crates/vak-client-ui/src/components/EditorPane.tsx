import { createEffect, createMemo, createSignal, Match, Show, Switch } from "solid-js";
import { activeId, editorPath } from "../store";
import * as api from "../api";
import CodeEditor from "./CodeEditor";

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

export default function EditorPane() {
  const [path, setPath] = createSignal<string | null>(null);
  const [file, setFile] = createSignal<api.FileResponse | null>(null);
  const [content, setContent] = createSignal("");
  const [savedContent, setSavedContent] = createSignal("");
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [saving, setSaving] = createSignal(false);
  let ta!: HTMLTextAreaElement;

  const dirty = createMemo(() => content() !== savedContent());

  createEffect(() => {
    const p = editorPath();
    const sid = activeId();
    if (!p || !sid) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    api
      .readFile(p, sid)
      .then((res) => {
        if (cancelled) return;
        setPath(p);
        setFile(res);
        // Only text has content; an image or binary is shown, never edited.
        setContent(res.content ?? "");
        setSavedContent(res.content ?? "");
      })
      .catch((e) => !cancelled && setError(e instanceof Error ? e.message : String(e)))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  });

  const save = async () => {
    const p = path();
    const sid = activeId();
    if (!p || !sid || saving()) return;
    // Conflict check: someone (agent or terminal) may have written the file
    // since we opened it. Refetch and warn instead of clobbering.
    try {
      const fresh = await api.readFile(p, sid);
      if (fresh.content !== savedContent()) {
        const overwrite = window.confirm(
          `"${p}" changed on disk since you opened it.\nOverwrite with your version?`,
        );
        if (!overwrite) return;
      }
      setSaving(true);
      await api.writeFile(p, content(), sid);
      setSavedContent(content());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
      e.preventDefault();
      void save();
    }
    if (e.key === "Tab") {
      e.preventDefault();
      const el = e.currentTarget as HTMLTextAreaElement;
      const { selectionStart: s, selectionEnd: en, value } = el;
      // Detect the indentation of the current line to match existing style.
      const lineStart = value.lastIndexOf("\n", s - 1) + 1;
      const lineText = value.slice(lineStart, s);
      const match = lineText.match(/^( +)/);
      const indent = match ? match[1] : "  ";
      const next = `${value.slice(0, s)}${indent}${value.slice(en)}`;
      setContent(next);
      queueMicrotask(() => {
        el.selectionStart = el.selectionEnd = s + indent.length;
      });
    }
  };

  return (
    <div class="editpane">
      <div class="dock-head">
        <Show when={path()} fallback={<span>Editor</span>}>
          {(p) => (
            <>
              <code class="ep-path" title={p()}>
                {p()}
              </code>
              <Show when={dirty()}>
                <span class="badge dirty">unsaved</span>
              </Show>
            </>
          )}
        </Show>
        <Show when={file() && !file()!.editable}>
          <span class="badge">{file()!.kind} · {formatBytes(file()!.bytes)}</span>
        </Show>
        <Show when={!file() || file()!.editable}>
          <button
            class="chip sm"
            onClick={() => void save()}
            disabled={!path() || !dirty() || saving()}
            title="Save (⌘S)"
          >
            {saving() ? "…" : "save"}
          </button>
        </Show>
      </div>
      <Show when={!error()} fallback={<div class="dock-empty">{error()}</div>}>
        <Show when={path()} fallback={<div class="dock-empty">Open a file from chat or the diff pane.</div>}>
          <Switch>
            <Match when={file()?.kind === "image"}>
              <div class="ep-image">
                <img src={file()!.data_url} alt={path() ?? "image"} />
              </div>
            </Match>
            <Match when={file()?.kind === "binary"}>
              <div class="dock-empty">
                Binary file — {formatBytes(file()!.bytes)}. Not shown, and not editable
                here so it cannot be corrupted.
              </div>
            </Match>
            <Match when={true}>
              <CodeEditor
                path={path()!}
                value={content()}
                onInput={setContent}
                onKeyDown={onKeyDown}
                ref={(el) => (ta = el)}
              />
            </Match>
          </Switch>
        </Show>
      </Show>
    </div>
  );
}
