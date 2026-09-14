import { createEffect, createSignal, onCleanup, Show } from "solid-js";
import Icon from "../Icon";
import { openComponentPreview, openArtifactCanvas } from "../../store";
import * as api from "../../api";
import { sandboxedSrcdoc } from "../../safeUrl";
import { artifactPreviewHtml } from "../../artifactPreview";

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
  const [previewHtml, setPreviewHtml] = createSignal("");
  let request = 0;
  onCleanup(() => { request += 1; });
  const [copied, setCopied] = createSignal(false);
  const [copyFailed, setCopyFailed] = createSignal(false);

  const path = () => props.data.artifact_path ?? "";

  const loadContent = async () => {
    const generation = ++request;
    const p = path();
    const inline = props.data.html;
    setLoading(true);
    setError(null);
    try {
      const html = inline ?? (p ? (await api.readFile(p)).content : undefined);
      if (html === undefined) throw new Error("Preview file is unavailable. Reload to try again.");
      const prepared = p
        ? await artifactPreviewHtml(p, html, props.data.connect_src)
        : sandboxedSrcdoc(html, props.data.connect_src ?? "'none'");
      if (generation !== request) return;
      setHtmlContent(html);
      setPreviewHtml(prepared);
    } catch (e) {
      if (generation === request) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (generation === request) setLoading(false);
    }
  };

  createEffect(() => {
    void path();
    void loadContent();
  });

  const reloadPreview = () => {
    void loadContent();
  };

  const previewPayload = () => ({
    id: props.data.preview_id ?? path(),
    title: props.data.title ?? "Component Preview",
    artifactPath: path(),
    html: props.data.html || (htmlContent().trim().length > 0 ? htmlContent() : undefined),
    previewId: props.data.preview_id,
    sandbox: props.data.sandbox,
    connectSrc: props.data.connect_src,
    timestamp: Date.now(),
  });

  const openInCanvas = () => {
    openArtifactCanvas(previewPayload());
  };

  const openInDock = () => {
    openComponentPreview(previewPayload());
  };

  const handleCopySource = async () => {
    setCopyFailed(false);
    try {
      await navigator.clipboard.writeText(htmlContent());
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      setCopied(false);
      setCopyFailed(true);
    }
  };

  const title = () => props.data.title || "Interactive Component Preview";

  return (
    <div class="canvas-card ui-preview-card">
      <div class="card-header">
        <div class="card-title-group">
          <span class="card-badge badge-indigo">UI Preview</span>
          <strong style="font-size: 12.5px; color: var(--text);">{title()}</strong>

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
            onClick={openInDock}
            title="Open in dock panel"
          >
            Dock
          </button>
          <button
            class="open-canvas-btn"
            onClick={openInCanvas}
            title="Open immersive canvas preview"
          >
            <Icon name="preview" size={14} /> Open Canvas
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
            style="position: relative; width: 100%; height: 390px; background: var(--surface); overflow: hidden; border-top: 1px solid var(--border-soft);"
          >
            <iframe
              srcdoc={previewHtml()}
              title={title()}
              sandbox={props.data.sandbox ?? "allow-scripts"}
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
              <button type="button" class="pill-action-btn" onClick={handleCopySource}>
                {copied() ? "✓ Copied" : copyFailed() ? "Copy failed" : "Copy Source"}
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
