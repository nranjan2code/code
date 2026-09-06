import { createEffect, createSignal, Show } from "solid-js";
import Icon from "../Icon";
import { openComponentPreview } from "../../store";
import * as api from "../../api";

export interface UIPreviewData {
  status?: string;
  preview_id?: string;
  title?: string;
  artifact_path?: string;
  sandbox?: string;
  connect_src?: string;
  html?: string;
}

export default function UIPreviewCard(props: { data: UIPreviewData }) {
  const [viewMode, setViewMode] = createSignal<"preview" | "source">("preview");
  const [htmlContent, setHtmlContent] = createSignal<string>(props.data.html ?? "");
  const [loading, setLoading] = createSignal(!props.data.html);
  const [error, setError] = createSignal<string | null>(null);
  const [reloadKey, setReloadKey] = createSignal(0);
  const [copied, setCopied] = createSignal(false);

  const path = () => props.data.artifact_path ?? "";

  const loadContent = async () => {
    if (props.data.html) {
      setHtmlContent(props.data.html);
      setLoading(false);
      return;
    }
    const p = path();
    if (!p) {
      setError("No artifact path or HTML content provided.");
      setLoading(false);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const res = await api.readFile(p);
      if (res.content !== undefined) {
        setHtmlContent(res.content);
      } else {
        setError("Unable to read preview artifact content.");
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  };

  createEffect(() => {
    void path();
    void loadContent();
  });

  const reloadPreview = () => {
    setReloadKey((k) => k + 1);
    void loadContent();
  };

  const openInDock = () => {
    openComponentPreview({
      id: props.data.preview_id ?? path(),
      title: props.data.title ?? "Component Preview",
      artifactPath: path(),
      html: htmlContent(),
      previewId: props.data.preview_id,
      sandbox: props.data.sandbox,
      connectSrc: props.data.connect_src,
      timestamp: Date.now(),
    });
  };

  const openInNewWindow = () => {
    const html = htmlContent();
    if (!html) return;
    const blob = new Blob([html], { type: "text/html" });
    const url = URL.createObjectURL(blob);
    window.open(url, "_blank", "noopener,noreferrer");
  };

  const handleCopySource = () => {
    void navigator.clipboard.writeText(htmlContent());
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };

  const title = () => props.data.title || "Interactive Component Preview";
  const connectStatus = () => props.data.connect_src ?? "blocked";

  return (
    <div class="canvas-card ui-preview-card">
      <div class="card-header">
        <div class="card-title-group">
          <span class="card-badge badge-indigo">UI Preview</span>
          <strong style="font-size: 12.5px; color: var(--text);">{title()}</strong>
          <span class="card-subtitle">
            <Show when={props.data.preview_id}>
              <code>{props.data.preview_id}</code> ·{" "}
            </Show>
            <span
              style="font-size: 10.5px; opacity: 0.85;"
              title={`Network connect-src is ${connectStatus()}`}
            >
              net: {connectStatus()}
            </span>
          </span>
        </div>
        <div class="card-actions" style="display: flex; gap: 6px; align-items: center;">
          <button
            class="pill-action-btn"
            classList={{ active: viewMode() === "source" }}
            onClick={() => setViewMode((m) => (m === "preview" ? "source" : "preview"))}
            title="Toggle between live preview and source markup"
          >
            {viewMode() === "preview" ? "Source" : "Preview"}
          </button>
          <button
            class="pill-action-btn"
            onClick={reloadPreview}
            title="Reload live preview"
          >
            Reload
          </button>
          <button
            class="pill-action-btn"
            onClick={openInNewWindow}
            title="Open preview in a standalone window"
          >
            Popout
          </button>
          <button
            class="pill-action-btn"
            style="background: color-mix(in srgb, var(--accent-bright) 16%, transparent); color: var(--accent-bright); border-color: color-mix(in srgb, var(--accent-bright) 30%, transparent);"
            onClick={openInDock}
            title="Open in Right Bar Preview Pane"
          >
            <Icon name="preview" size={12} /> Right Bar
          </button>
        </div>
      </div>

      <Show when={loading()}>
        <div style="padding: 24px; text-align: center; color: var(--muted); font-size: 12px;">
          Loading sandboxed preview artifact…
        </div>
      </Show>

      <Show when={error()}>
        <div style="padding: 16px; color: var(--red); background: color-mix(in srgb, var(--red) 8%, transparent); font-size: 11.5px;">
          Failed to load preview: {error()}
        </div>
      </Show>

      <Show when={!loading() && !error()}>
        <Show when={viewMode() === "preview"}>
          <div
            class="ui-preview-stage"
            style="position: relative; width: 100%; height: 390px; background: #0b0d13; overflow: hidden; border-top: 1px solid var(--border-soft);"
          >
            <iframe
              srcdoc={htmlContent()}
              title={title()}
              sandbox="allow-scripts"
              style="width: 100%; height: 100%; border: 0; display: block;"
            />
          </div>
        </Show>

        <Show when={viewMode() === "source"}>
          <div style="padding: 10px 14px; background: var(--bg); border-top: 1px solid var(--border-soft);">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px;">
              <span style="font-size: 10.5px; color: var(--faint); font-family: var(--mono);">
                Quarantined: {path()}
              </span>
              <button class="pill-action-btn" onClick={handleCopySource}>
                {copied() ? "✓ Copied" : "Copy Source"}
              </button>
            </div>
            <pre
              style="margin: 0; max-height: 280px; overflow: auto; font-family: var(--mono); font-size: 11px; line-height: 1.55; color: var(--text-soft); padding: 8px; border-radius: 6px; background: var(--surface);"
            >
              <code>{htmlContent()}</code>
            </pre>
          </div>
        </Show>
      </Show>
    </div>
  );
}
