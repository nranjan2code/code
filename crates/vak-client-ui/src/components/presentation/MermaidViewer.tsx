import { createEffect, createSignal, Show } from "solid-js";
import type { JSX } from "solid-js";
import { uiPreferences } from "../../store";

let renderIdSeq = 0;

/** Diagram colours come from the live theme tokens, so every theme (and a
 * future one) is covered without a second palette to keep in step. */
function tokenThemeVariables(): Record<string, string | boolean> {
  const style = getComputedStyle(document.documentElement);
  const token = (name: string) => style.getPropertyValue(name).trim();
  return {
    darkMode: style.colorScheme.includes("dark"),
    background: token("--surface"),
    primaryColor: token("--surface-raised"),
    primaryTextColor: token("--text"),
    primaryBorderColor: token("--border-strong"),
    lineColor: token("--muted"),
    secondaryColor: token("--accent-wash"),
    tertiaryColor: token("--bg"),
    textColor: token("--text"),
    noteBkgColor: token("--live-wash"),
    noteTextColor: token("--live-ink"),
  };
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
      mermaid.initialize({
        startOnLoad: false,
        theme: "base",
        securityLevel: "strict",
        fontFamily: 'var(--sans), -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
        themeVariables: tokenThemeVariables(),
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
