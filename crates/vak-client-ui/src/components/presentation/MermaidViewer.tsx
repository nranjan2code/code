import { createEffect, createSignal, Show } from "solid-js";
import type { JSX } from "solid-js";
import { uiPreferences } from "../../store";

let renderIdSeq = 0;

function getMermaidTheme(): "dark" | "neutral" | "base" | "default" {
  const theme = uiPreferences.theme === "system"
    ? (typeof window !== "undefined" && window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light")
    : uiPreferences.theme;
  if (theme === "light") return "neutral";
  if (theme === "contrast") return "base";
  return "dark";
}

export default function MermaidViewer(props: { source: string; title?: string }): JSX.Element {
  const [viewMode, setViewMode] = createSignal<"diagram" | "source">("diagram");
  const [svgHtml, setSvgHtml] = createSignal<string | null>(null);
  const [errorMsg, setErrorMsg] = createSignal<string | null>(null);
  const [copied, setCopied] = createSignal(false);
  const [copyFailed, setCopyFailed] = createSignal(false);
  const [isRendering, setIsRendering] = createSignal(true);

  let containerRef!: HTMLDivElement;

  const renderDiagram = async () => {
    setIsRendering(true);
    setErrorMsg(null);
    try {
      const mermaid = (await import("mermaid")).default;
      const theme = getMermaidTheme();
      mermaid.initialize({
        startOnLoad: false,
        theme,
        securityLevel: "strict",
        fontFamily: 'var(--sans), -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
        themeVariables: theme === "neutral" ? {
          primaryColor: "#f0ede5",
          primaryTextColor: "#23211c",
          primaryBorderColor: "#bdb5a4",
          lineColor: "#635f54",
          secondaryColor: "#e6e1d6",
          tertiaryColor: "#faf8f3",
          background: "#faf8f3",
        } : theme === "base" ? {
          primaryColor: "#181818",
          primaryTextColor: "#ffffff",
          primaryBorderColor: "#ffffff",
          lineColor: "#ffffff",
          background: "#080808",
        } : {
          primaryColor: "#1e2236",
          primaryTextColor: "#ecebf5",
          primaryBorderColor: "#3d4466",
          lineColor: "#a3adf7",
          secondaryColor: "#252a40",
          tertiaryColor: "#171a2b",
          background: "#0f1120",
        },
      });

      const uniqueId = `mermaid-diag-${++renderIdSeq}`;
      const trimmed = props.source.trim();
      const { svg } = await mermaid.render(uniqueId, trimmed);
      setSvgHtml(svg);
    } catch (err: any) {
      setErrorMsg(err?.message || "Failed to render diagram");
      setSvgHtml(null);
    } finally {
      setIsRendering(false);
    }
  };

  createEffect(() => {
    // Re-render when source or theme preference changes
    void props.source;
    void uiPreferences.theme;
    void renderDiagram();
  });

  const handleCopySource = async () => {
    setCopyFailed(false);
    try {
      await navigator.clipboard.writeText(props.source);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      setCopied(false);
      setCopyFailed(true);
    }
  };

  const handleDownloadSvg = () => {
    const svg = svgHtml();
    if (!svg) return;
    const blob = new Blob([svg], { type: "image/svg+xml;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `diagram-${Date.now()}.svg`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  };

  return (
    <div class="canvas-card mermaid-canvas-wrap">
      <div class="card-header">
        <div class="card-title-group">
          <span class="card-badge badge-indigo">Mermaid Diagram</span>
          <span class="card-subtitle">{props.title ?? "Architecture & Flow"}</span>
        </div>
        <div class="card-actions">
          <div class="view-toggle-group">
            <button
              class="pill-action-btn"
              classList={{ active: viewMode() === "diagram" }}
              onClick={() => setViewMode("diagram")}
            >
              Diagram
            </button>
            <button
              class="pill-action-btn"
              classList={{ active: viewMode() === "source" }}
              onClick={() => setViewMode("source")}
            >
              Source
            </button>
          </div>
          <button type="button" class="pill-action-btn" onClick={handleCopySource}>
            {copied() ? "✓ Copied" : copyFailed() ? "Copy failed" : "Copy Source"}
          </button>
          <Show when={svgHtml() && viewMode() === "diagram"}>
            <button class="pill-action-btn" onClick={handleDownloadSvg}>
              Save SVG
            </button>
          </Show>
        </div>
      </div>

      <div class="mermaid-body">
        <Show when={viewMode() === "diagram"}>
          <Show when={isRendering()}>
            <div class="mermaid-loading">Rendering diagram…</div>
          </Show>
          <Show when={errorMsg()}>
            <div class="mermaid-error">
              <strong>Diagram syntax error</strong>
              <p>{errorMsg()}</p>
              <pre class="mermaid-raw-source">{props.source}</pre>
            </div>
          </Show>
          <Show when={svgHtml() && !errorMsg()}>
            <div
              class="mermaid-svg-stage"
              ref={containerRef}
              innerHTML={svgHtml()!}
            />
          </Show>
        </Show>

        <Show when={viewMode() === "source"}>
          <pre class="mermaid-source-pre"><code>{props.source}</code></pre>
        </Show>
      </div>
    </div>
  );
}
