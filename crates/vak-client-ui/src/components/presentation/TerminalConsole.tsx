import { Show, createSignal } from "solid-js";

export interface TerminalData {
  command?: string;
  output: string;
  exit_code?: number;
  duration_ms?: number;
}

export default function TerminalConsole(props: { data: TerminalData }) {
  const [copied, setCopied] = createSignal(false);
  const [copyError, setCopyError] = createSignal(false);

  const handleCopy = async () => {
    setCopyError(false);
    try {
      await navigator.clipboard.writeText(props.data.output);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      setCopyError(true);
    }
  };

  const isSuccess = () => props.data.exit_code === 0;

  return (
    <div class="canvas-card terminal-wrap">
      <div class="card-header">
        <div class="card-title-group">
          <span class={`card-badge ${isSuccess() ? "badge-indigo" : "badge-rose"}`}>
            Terminal
          </span>
          <span class="card-subtitle">
            {props.data.exit_code === undefined ? "Exit status unavailable" : `Exit ${props.data.exit_code}`}
            <Show when={props.data.duration_ms !== undefined}>
              {" "}· {props.data.duration_ms}ms
            </Show>
          </span>
        </div>
        <div class="card-actions">
          <button type="button" class="pill-action-btn" onClick={handleCopy}>
            {copied() ? "✓ Copied" : copyError() ? "Copy failed" : "Copy Output"}
          </button>
        </div>
      </div>

      <div class="sleek-terminal">
        <Show when={props.data.command}>
          <div>
            <span class="term-line-cmd">$</span> {props.data.command}
          </div>
        </Show>
        <pre class="term-output-text">{props.data.output}</pre>
      </div>
    </div>
  );
}
