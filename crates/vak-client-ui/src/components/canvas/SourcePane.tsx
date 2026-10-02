import { createSignal, For, onCleanup, Show } from "solid-js";
import Icon from "../Icon";
import { technicalDetails } from "../../store";
import type { Selection } from "../../canvasSelection";

/** Source text with selectable lines; a selection is what a comment is about.
 *  The file is named by its name; its path shows under Show technical details. */
export default function SourcePane(props: {
  text: string;
  label: string;
  path?: string;
  selection: Selection | null;
  onSelect: (selection: Selection | null) => void;
}) {
  const [copied, setCopied] = createSignal(false);
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(copiedTimer));
  const lines = () => props.text.split("\n");
  const range = () => (props.selection?.kind === "lines" ? props.selection : null);
  const selected = (line: number) => {
    const current = range();
    return !!current && line >= current.start && line <= (current.end ?? current.start);
  };
  const select = (line: number, extend: boolean) => {
    const current = range();
    if (extend && current) props.onSelect({ kind: "lines", start: Math.min(current.start, line), end: Math.max(current.end ?? current.start, line) });
    else props.onSelect({ kind: "lines", start: line });
  };
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(props.text);
      setCopied(true);
      clearTimeout(copiedTimer);
      copiedTimer = setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard may not be available in all contexts
    }
  };
  return (
    <div class="artifact-canvas-source">
      <div class="artifact-canvas-source-head">
        <span class="artifact-canvas-source-path">
          {props.label}
          <Show when={technicalDetails() && props.path && props.path !== props.label}><small>{props.path}</small></Show>
        </span>
        <button type="button" class="artifact-canvas-btn" onClick={copy}>
          <Icon name="copy" size={13} />
          {copied() ? "Copied" : "Copy"}
        </button>
      </div>
      <div class="artifact-canvas-code" role="list" aria-label="Source lines">
        <For each={lines()}>{(line, index) => {
          const number = index() + 1;
          return <div class="artifact-canvas-code-line" classList={{ selected: selected(number) }} role="listitem">
            <button type="button" class="artifact-canvas-line-number" aria-label={`Select line ${number}`} aria-pressed={selected(number)} onClick={(event) => select(number, event.shiftKey)}>{number}</button>
            <code>{line || " "}</code>
          </div>;
        }}</For>
      </div>
    </div>
  );
}
