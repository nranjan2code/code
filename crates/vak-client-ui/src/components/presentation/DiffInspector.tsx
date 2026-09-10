import { For, Show, createMemo, createSignal } from "solid-js";
import { openInEditor } from "../../store";

export interface DiffFile {
  filename: string;
  additions: number;
  deletions: number;
  hunks: string;
}

export interface DiffInspectorData {
  files: DiffFile[];
}

export default function DiffInspector(props: {
  data?: DiffInspectorData;
  rawDiff?: string;
  filename?: string;
}) {
  const [activeIdx, setActiveIdx] = createSignal(0);
  const [splitMode, setSplitMode] = createSignal(false);
  const [copied, setCopied] = createSignal(false);

  const files = createMemo<DiffFile[]>(() => {
    if (props.data?.files && props.data.files.length > 0) {
      return props.data.files;
    }
    const raw = props.rawDiff ?? "";
    const adds = (raw.match(/^\+[^+]/gm) || []).length;
    const dels = (raw.match(/^-[^-]/gm) || []).length;
    return [
      {
        filename: props.filename ?? "changes.diff",
        additions: adds,
        deletions: dels,
        hunks: raw,
      },
    ];
  });

  const activeFile = () => files()[activeIdx()] ?? files()[0];

  const parsedLines = createMemo(() => {
    const text = activeFile()?.hunks ?? "";
    const lines = text.split("\n");
    let oldLine = 1;
    let newLine = 1;

    return lines.map((line) => {
      if (line.startsWith("@@")) {
        const match = /@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
        if (match) {
          oldLine = parseInt(match[1], 10);
          newLine = parseInt(match[2], 10);
        }
        return { type: "hunk" as const, text: line, oldNum: "...", newNum: "..." };
      }
      if (line.startsWith("+") && !line.startsWith("+++")) {
        const item = { type: "add" as const, text: line, oldNum: "", newNum: String(newLine) };
        newLine++;
        return item;
      }
      if (line.startsWith("-") && !line.startsWith("---")) {
        const item = { type: "del" as const, text: line, oldNum: String(oldLine), newNum: "" };
        oldLine++;
        return item;
      }
      const item = { type: "normal" as const, text: line, oldNum: String(oldLine), newNum: String(newLine) };
      oldLine++;
      newLine++;
      return item;
    });
  });

  const handleCopy = () => {
    void navigator.clipboard.writeText(activeFile()?.hunks ?? "")
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1200);
      })
      .catch(() => setCopied(false));
  };

  const handleOpen = () => {
    const fn = activeFile()?.filename;
    if (fn) openInEditor(fn);
  };

  return (
    <div class="canvas-card diff-inspector-wrap">
      <div class="card-header">
        <div class="card-title-group">
          <span class="card-badge badge-indigo">Diff Inspector</span>
          <span class="card-subtitle">{activeFile()?.filename}</span>
        </div>
        <div class="card-actions">
          <button type="button" class="pill-action-btn" onClick={() => setSplitMode(!splitMode())}>
            {splitMode() ? "Unified" : "Side-by-Side"}
          </button>
          <button type="button" class="pill-action-btn" onClick={handleCopy}>
            {copied() ? "✓ Copied" : "Copy Diff"}
          </button>
          <button type="button" class="pill-action-btn" onClick={handleOpen}>
            Open in Editor
          </button>
        </div>
      </div>

      <div class="diff-workspace">
        <Show when={files().length > 1}>
          <div class="diff-file-sidebar">
            <For each={files()}>
              {(f, idx) => (
                <div
                  class="diff-file-item"
                  classList={{ active: idx() === activeIdx() }}
                  onClick={() => setActiveIdx(idx())}
                >
                  <span class="diff-file-name">{f.filename}</span>
                  <span class="diff-delta-pill">
                    <span style={{ color: "var(--emerald-bright)" }}>+{f.additions}</span>{" "}
                    <span style={{ color: "var(--rose-bright)" }}>-{f.deletions}</span>
                  </span>
                </div>
              )}
            </For>
          </div>
        </Show>

        <div class="diff-code-area" classList={{ "split-mode": splitMode() }}>
          <For each={parsedLines()}>
            {(row) => (
              <div class={`diff-row ${row.type}`}>
                <span class="diff-num">{row.oldNum || " "}</span>
                <span class="diff-num">{row.newNum || " "}</span>
                <span class="diff-line-text">{row.text}</span>
              </div>
            )}
          </For>
        </div>
      </div>
    </div>
  );
}
