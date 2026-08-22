import { createEffect, createMemo, createSignal, Show } from "solid-js";
import { editorPath } from "../store";
import * as api from "../api";

export default function EditorPane() {
  const [path, setPath] = createSignal<string | null>(null);
  const [content, setContent] = createSignal("");
  const [savedContent, setSavedContent] = createSignal("");
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [saving, setSaving] = createSignal(false);
  let ta!: HTMLTextAreaElement;

  const dirty = createMemo(() => content() !== savedContent());

  createEffect(() => {
    const p = editorPath();
    if (!p) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    api
      .readFile(p)
      .then((res) => {
        if (cancelled) return;
        setPath(p);
        setContent(res.content);
        setSavedContent(res.content);
      })
      .catch((e) => !cancelled && setError(e instanceof Error ? e.message : String(e)))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  });

  const save = async () => {
    const p = path();
    if (!p || saving()) return;
    // Conflict check: someone (agent or terminal) may have written the file
    // since we opened it. Refetch and warn instead of clobbering.
    try {
      const fresh = await api.readFile(p);
      if (fresh.content !== savedContent()) {
        const overwrite = window.confirm(
          `"${p}" changed on disk since you opened it.\nOverwrite with your version?`,
        );
        if (!overwrite) return;
      }
      setSaving(true);
      await api.writeFile(p, content());
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
      const next = `${value.slice(0, s)}  ${value.slice(en)}`;
      setContent(next);
      queueMicrotask(() => {
        el.selectionStart = el.selectionEnd = s + 2;
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
        <button
          class="chip sm"
          onClick={() => void save()}
          disabled={!path() || !dirty() || saving()}
          title="Save (⌘S)"
        >
          {saving() ? "…" : "save"}
        </button>
      </div>
      <Show when={!error()} fallback={<div class="dock-empty">{error()}</div>}>
        <Show when={path()} fallback={<div class="dock-empty">Open a file from chat or the diff pane.</div>}>
          <textarea
            ref={ta}
            class="ep-text"
            spellcheck={false}
            value={content()}
            onInput={(e) => setContent(e.currentTarget.value)}
            onKeyDown={onKeyDown}
          />
        </Show>
      </Show>
    </div>
  );
}
