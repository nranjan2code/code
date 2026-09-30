import { createSignal, For } from "solid-js";
import Icon from "../Icon";
import type { LineSelection } from "../../canvasStack";

/** Source text with selectable lines; a selection is what a comment is about. */
export default function SourcePane(props: {
  text: string;
  label: string;
  selection: LineSelection | null;
  onSelect: (selection: LineSelection | null) => void;
}) {
  const [copied, setCopied] = createSignal(false);
  const lines = () => props.text.split("\n");
  const selected = (line: number) => {
    const range = props.selection;
    return !!range && line >= range.start && line <= (range.end ?? range.start);
  };
  const select = (line: number, extend: boolean) => {
    const range = props.selection;
    if (extend && range) props.onSelect({ start: Math.min(range.start, line), end: Math.max(range.end ?? range.start, line) });
    else props.onSelect({ start: line });
  };
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(props.text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard may not be available in all contexts
    }
  };
  return (
    <div class="artifact-canvas-source">
      <div class="artifact-canvas-source-head">
        <span class="artifact-canvas-source-path">{props.label}</span>
        <button type="button" class="artifact-canvas-btn" onClick={copy}>
          <Icon name="copy" size={13} />
          {copied() ? "Copied!" : "Copy"}
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
