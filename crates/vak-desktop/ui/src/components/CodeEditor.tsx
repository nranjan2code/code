import { createEffect, createSignal, onCleanup } from "solid-js";
import { highlight, languageFor } from "../highlight";

/**
 * An editable, syntax-coloured text view.
 *
 * A transparent textarea sits exactly on top of highlighted output: the user
 * edits real text (selection, caret, undo, IME all behave natively) while the
 * colour comes from the layer behind. The two layers must agree on font,
 * size, line-height, padding and wrapping to the pixel, which is why those
 * live in one CSS rule shared by both.
 */
export default function CodeEditor(props: {
  path: string;
  value: string;
  onInput: (next: string) => void;
  onKeyDown: (e: KeyboardEvent) => void;
  ref?: (el: HTMLTextAreaElement) => void;
}) {
  const [html, setHtml] = createSignal<string | null>(null);
  let ta!: HTMLTextAreaElement;
  let pre!: HTMLDivElement;

  // Re-highlight on content or language change. Highlighting is async, so a
  // stale result must never overwrite a newer one.
  createEffect(() => {
    const code = props.value;
    const lang = languageFor(props.path);
    let cancelled = false;
    // Very large files are left plain: tokenising megabytes blocks the UI
    // for longer than the colour is worth.
    if (code.length > 400_000) {
      setHtml(null);
      return;
    }
    void highlight(code, lang).then((out) => {
      if (!cancelled) setHtml(out);
    });
    onCleanup(() => {
      cancelled = true;
    });
  });

  const syncScroll = () => {
    if (!pre || !ta) return;
    pre.scrollTop = ta.scrollTop;
    pre.scrollLeft = ta.scrollLeft;
  };

  return (
    <div class="ce">
      <div class="ce-hl" ref={pre} aria-hidden="true" innerHTML={html() ?? undefined}>
        {/* Plain fallback when no grammar matched, so text still lines up. */}
        {html() ? null : <pre class="shiki"><code>{props.value}</code></pre>}
      </div>
      <textarea
        ref={(el) => {
          ta = el;
          props.ref?.(el);
        }}
        class="ce-ta"
        classList={{ plain: !html() }}
        spellcheck={false}
        value={props.value}
        onInput={(e) => props.onInput(e.currentTarget.value)}
        onKeyDown={props.onKeyDown}
        onScroll={syncScroll}
      />
    </div>
  );
}
