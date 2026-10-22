import { For, Show, createEffect, createMemo, createSignal, createUniqueId, onCleanup, useContext } from "solid-js";
import type { AdaptiveRenderNode } from "../../types";
import { previewSandbox, safeUrl, sandboxedSrcdoc } from "../../safeUrl";
import { activeId, openInEditor, openInlineCanvas, openArtifactFile, uiPreferences } from "../../store";
import { inlineTitle } from "../../canvasSubject";
import { artifactPreviewHtml } from "../../artifactPreview";
import * as api from "../../api";
import Icon from "../Icon";
import { PresentationPageResourceContext, PresentationSessionContext, records, type Payload } from "./shared";
import { themedPreview } from "../../previewTheme";
import { withPreviewLibraries } from "../../previewLibraries";
import { mediaSource } from "../../mediaSource";

// The code and media forms: each reads its stored payload through a render
// node, the same shape a pack's tree carries.
type RenderSurface = "full" | "compact";

function str(props: Record<string, unknown>, key: string): string | undefined {
  const value = props[key];
  return typeof value === "string" ? value : typeof value === "number" ? String(value) : undefined;
}

function num(props: Record<string, unknown>, key: string): number | undefined {
  const value = props[key];
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

const nodeOf = (primitive: string, payload: Payload, list?: string): AdaptiveRenderNode => ({
  primitive,
  props: payload,
  children: list ? records(payload[list]).map((props) => ({ primitive: "item", props, children: [] })) : [],
});

export const DiffForm = (props: { payload: Payload }) => renderDiff(nodeOf("diff", props.payload, "files"), "full");
export const TerminalForm = (props: { payload: Payload }) => renderTerminal(nodeOf("terminal", props.payload), "full");
export const TestReportForm = (props: { payload: Payload }) => renderTestMatrix(nodeOf("test_report", props.payload, "tests"), "full");
export const PreviewForm = (props: { payload: Payload }) => renderUiPreview(nodeOf("preview", props.payload), "full");
export const MediaForm = (props: { payload: Payload }) =>
  renderMedia(nodeOf("media", { ...props.payload, kind: props.payload.format, source: props.payload.url }), "full");
export const LinkForm = (props: { payload: Payload }) => renderMedia(nodeOf("media", { ...props.payload, kind: "link" }), "full");

function renderDiff(node: AdaptiveRenderNode, surface: RenderSurface) {
  const compact = surface === "compact";
  const [activeIdx, setActiveIdx] = createSignal(0);
  const [splitMode, setSplitMode] = createSignal(false);
  const [copied, setCopied] = createSignal(false);

  const files = () => (Array.isArray(node.children) ? node.children : []).map((child) =>
    child && typeof child === "object" && child.props && typeof child.props === "object" ? child.props : {},
  );
  const activeFile = () => files()[activeIdx()] ?? files()[0];

  const parsedLines = createMemo(() => {
    const text = str(activeFile() ?? {}, "hunks") ?? "";
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
    void navigator.clipboard
      .writeText(str(activeFile() ?? {}, "hunks") ?? "")
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1200);
      })
      .catch(() => setCopied(false));
  };

  const handleOpen = () => {
    const fn = str(activeFile() ?? {}, "filename");
    if (fn) openInEditor(fn);
  };

  return (
    <div class="canvas-card diff-inspector-wrap" classList={{ "diff-inspector-wrap-compact": compact }}>
      <div class="card-header">
        <div class="card-title-group">
          <Show when={str(activeFile() ?? {}, "filename")}><span class="card-subtitle">{str(activeFile() ?? {}, "filename")}</span></Show>
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
                <div class="diff-file-item" classList={{ active: idx() === activeIdx() }} onClick={() => setActiveIdx(idx())}>
                  <span class="diff-file-name">{str(f, "filename")}</span>
                  <span class="diff-delta-pill">
                    <span class="diff-count-add">+{num(f, "additions") ?? 0}</span>{" "}
                    <span class="diff-count-del">-{num(f, "deletions") ?? 0}</span>
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

/**
 * `terminal` primitive (maps to `vak_presentation::Primitive::Terminal`).
 * Mirrors the legacy TerminalConsole exactly. The old component is a plain
 * static transcript view (no xterm.js — `@xterm/xterm` is only used by the
 * live TerminalPane), so there is no emulator to delegate to here.
 */
/**
 * How a command ended, in plain words: its exit code, or "Stopped" when the
 * runtime recorded a run with no exit (stopped at its limit or cancelled),
 * and how long it ran. Nothing when the card says neither.
 */
function terminalStatus(exitCode: number | undefined, durationMs: number | undefined): string {
  const ended = exitCode !== undefined ? `Exit ${exitCode}` : durationMs !== undefined ? "Stopped" : "";
  if (durationMs === undefined) return ended;
  const took = durationMs < 1000 ? `${durationMs} ms` : `${(durationMs / 1000).toFixed(1)} s`;
  return ended ? `${ended} · ${took}` : took;
}

function renderTerminal(node: AdaptiveRenderNode, surface: RenderSurface) {
  const compact = surface === "compact";
  const [copied, setCopied] = createSignal(false);
  const [copyError, setCopyError] = createSignal(false);
  const output = () => str(node.props, "output") ?? "";
  const command = () => str(node.props, "command");
  const exitCode = () => num(node.props, "exit_code");
  const durationMs = () => num(node.props, "duration_ms");

  const handleCopy = async () => {
    setCopyError(false);
    try {
      await navigator.clipboard.writeText(output());
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      setCopyError(true);
    }
  };


  return (
    <div class="canvas-card terminal-wrap" classList={{ "terminal-wrap-compact": compact }}>
      <div class="card-header">
        <div class="card-title-group">
          <span class="card-subtitle">{terminalStatus(exitCode(), durationMs())}</span>
        </div>
        <div class="card-actions">
          <button type="button" class="pill-action-btn" onClick={handleCopy}>
            {copied() ? "✓ Copied" : copyError() ? "Copy failed" : "Copy Output"}
          </button>
        </div>
      </div>

      <div class="sleek-terminal">
        <Show when={command()}>
          <div>
            <span class="term-line-cmd">$</span> {command()}
          </div>
        </Show>
        <pre class="term-output-text">{output()}</pre>
      </div>
    </div>
  );
}

/**
 * `test_matrix` primitive (maps to `vak_presentation::Primitive::TestMatrix`).
 * Mirrors the legacy TestMatrix: all/failed filter chips, a success-rate ring,
 * pass/fail counters and per-test cards with an optional traceback drawer.
 */
function renderTestMatrix(node: AdaptiveRenderNode, surface: RenderSurface) {
  const compact = surface === "compact";
  const [filter, setFilter] = createSignal<"all" | "failed">("all");

  const tests = () => (Array.isArray(node.children) ? node.children : []).map((child) =>
    child && typeof child === "object" && child.props && typeof child.props === "object" ? child.props : {},
  );

  const passedCount = createMemo(() => num(node.props, "passed") ?? tests().filter((t) => str(t, "status") === "passed").length);
  const failedCount = createMemo(() => num(node.props, "failed") ?? tests().filter((t) => str(t, "status") === "failed").length);
  const totalCount = createMemo(() => num(node.props, "total") ?? tests().length);
  const hasReportedOutcome = createMemo(() =>
    totalCount() > 0 && (passedCount() > 0 || failedCount() > 0 || tests().some((t) => {
      const status = str(t, "status");
      return status === "passed" || status === "failed" || status === "skipped";
    })),
  );
  const successPercent = createMemo(() => {
    const tot = totalCount();
    if (!hasReportedOutcome()) return 0;
    return Math.round((passedCount() / tot) * 100);
  });
  const filteredTests = createMemo(() =>
    filter() === "failed" ? tests().filter((t) => str(t, "status") === "failed") : tests(),
  );

  return (
    <div class="canvas-card test-matrix-wrap" classList={{ "test-matrix-wrap-compact": compact }}>
      <div class="card-header">
        <div class="card-title-group">
          <span class="card-subtitle">
            {[str(node.props, "suite_name") ?? str(node.props, "command"), terminalStatus(num(node.props, "exit_code"), num(node.props, "duration_ms"))]
              .filter(Boolean)
              .join(" · ")}
          </span>
        </div>
        <div class="test-tab-filter">
          <button class="filter-chip" classList={{ active: filter() === "all" }} onClick={() => setFilter("all")}>
            All ({totalCount()})
          </button>
          <button class="filter-chip" classList={{ active: filter() === "failed" }} onClick={() => setFilter("failed")}>
            Failed ({failedCount()})
          </button>
        </div>
      </div>

      <div class="test-dashboard">
        <div class="test-stats-row">
          <div class="test-progress-group">
            <div class="test-ring-wrapper">
              <svg viewBox="0 0 36 36">
                <path
                  d="M18 2.0845 a 15.9155 15.9155 0 0 1 0 31.831 a 15.9155 15.9155 0 0 1 0 -31.831"
                  fill="none"
                  stroke="var(--rose-wash)"
                  stroke-width="3.5"
                />
                <path
                  d="M18 2.0845 a 15.9155 15.9155 0 0 1 0 31.831 a 15.9155 15.9155 0 0 1 0 -31.831"
                  fill="none"
                  stroke={hasReportedOutcome() ? "var(--emerald-bright)" : "var(--muted)"}
                  stroke-width="3.5"
                  stroke-dasharray={`${successPercent()}, 100`}
                  stroke-linecap="round"
                />
              </svg>
            </div>
            <div>
              <div class="test-summary-headline">
                {hasReportedOutcome() ? `${successPercent()}% Success Rate` : totalCount() === 0 ? "No tests reported" : "Results unavailable"}
              </div>
              <div class="test-summary-sub">
                {passedCount()} of {totalCount()} tests verified
              </div>
            </div>
          </div>
          <div class="test-summary-counts">
            <Show when={hasReportedOutcome()}>
              <span class="test-count-pass">● {passedCount()} Passed</span>
            </Show>
            <Show when={failedCount() > 0}>
              <span class="test-count-fail">✕ {failedCount()} Failed</span>
            </Show>
          </div>
        </div>

        <div class="test-grid-list">
          <For each={filteredTests()}>
            {(t) => {
              const status = str(t, "status");
              const passed = status === "passed";
              const failed = status === "failed";
              const duration = num(t, "duration_ms");
              const detail = str(t, "traceback") ?? str(t, "message");
              return (
                <div class="test-item-card" classList={{ "fail-card": failed }}>
                  <div class="test-item-body">
                    <div class="test-item-head">
                      <div>
                        <span class="test-mark" classList={{ "is-fail": failed, "is-pass": passed }}>
                          {failed ? "✕" : passed ? "✓" : "•"}
                        </span>
                        <strong class="test-name">{str(t, "name")}</strong>
                      </div>
                      <Show when={duration !== undefined}>
                        <span class="test-duration">{duration}ms</span>
                      </Show>
                    </div>
                    <Show when={detail}>
                      <div class="traceback-drawer">{detail}</div>
                    </Show>
                  </div>
                </div>
              );
            }}
          </For>
        </div>
      </div>
    </div>
  );
}

/** File and markup previews open on demand, through the same isolation
 * pipeline as Canvas. A collapsed card never mounts an interactive frame. */
function renderUiPreview(node: AdaptiveRenderNode, surface: RenderSurface) {
  const compact = surface === "compact";
  const [expanded, setExpanded] = createSignal(false);
  const bodyId = createUniqueId();
  const pageReader = useContext(PresentationPageResourceContext);
  const session = pageReader ? undefined : useContext(PresentationSessionContext) ?? activeId() ?? undefined;
  const reader = pageReader ?? api.workspaceReader(session);
  const [viewMode, setViewMode] = createSignal<"preview" | "source">("preview");
  const inlineHtml = () => str(node.props, "html");
  const [htmlContent, setHtmlContent] = createSignal<string>(inlineHtml() ?? "");
  const [loading, setLoading] = createSignal(!inlineHtml());
  const [error, setError] = createSignal<string | null>(null);
  const [previewHtml, setPreviewHtml] = createSignal("");
  let request = 0;
  onCleanup(() => {
    request += 1;
  });
  const [copied, setCopied] = createSignal(false);
  const [copyFailed, setCopyFailed] = createSignal(false);

  const path = () => str(node.props, "artifact_path") ?? "";

  const loadContent = async () => {
    const generation = ++request;
    const p = path();
    const inline = inlineHtml();
    setLoading(true);
    setError(null);
    try {
      const html = inline ?? (p ? (await reader.readFile(p)).content : undefined);
      if (html === undefined) throw new Error("This page isn't available. Reload to try again.");
      const libraries = node.props.libraries;
      const prepared = p
        ? await artifactPreviewHtml(p, html, reader).catch(() => sandboxedSrcdoc(html))
        : sandboxedSrcdoc(Array.isArray(libraries) ? await withPreviewLibraries(html, libraries) : html);
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
    if (expanded()) void loadContent();
  });

  const reloadPreview = () => {
    void loadContent();
  };

  const openInCanvas = () => {
    const html = inlineHtml();
    const libraries = Array.isArray(node.props.libraries) ? node.props.libraries.filter((name): name is string => typeof name === "string") : undefined;
    if (html) void openInlineCanvas({ title: inlineTitle(html, str(node.props, "title")), html, basePath: path() || undefined, sessionId: session, libraries });
    else if (path()) openArtifactFile(path(), { sessionId: session });
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

  const title = () => str(node.props, "title")?.trim() || "Web preview";

  return (
    <div class="canvas-card ui-preview-card" classList={{ "ui-preview-card-compact": compact }}>
      <div class="card-header">
        <button type="button" class="ui-preview-toggle" aria-label={`${title()}: ${expanded() ? "Hide preview" : "Show preview"}`} title={expanded() ? "Hide preview" : "Show preview"} aria-expanded={expanded()} aria-controls={bodyId} onClick={() => setExpanded(!expanded())}>
          <span class="ui-preview-chevron" classList={{ expanded: expanded() }}><Icon name="chevron" size={16} /></span>
          <strong class="ui-preview-title">{title()}</strong>
        </button>
        <div class="card-actions ui-preview-actions">
          <Show when={expanded()}>
            <button
              type="button"
              class="file-action-icon"
              classList={{ active: viewMode() === "source" }}
              onClick={() => setViewMode((m) => (m === "preview" ? "source" : "preview"))}
              aria-label={viewMode() === "preview" ? "Show source" : "Show preview"}
              title={viewMode() === "preview" ? "Show the code" : "Show the page"}
            >
              <Icon name={viewMode() === "preview" ? "code" : "preview"} size={17} />
            </button>
            <button type="button" class="file-action-icon" onClick={reloadPreview} aria-label="Reload preview" title="Reload preview">
              <Icon name="sync" size={17} />
            </button>
          </Show>
          <button type="button" class="btn" onClick={openInCanvas} title="Open in Canvas">
            Open
          </button>
        </div>
      </div>

      <div id={bodyId}>
        <Show when={expanded()}>
          <Show when={loading()}>
            <div class="ui-preview-state">
              Loading preview…
            </div>
          </Show>

          <Show when={error()}>
            <div class="ui-preview-state is-error">
              Failed to load preview: {error()}
            </div>
          </Show>

          <Show when={!loading() && !error()}>
            <Show when={viewMode() === "preview"}>
              <div class="ui-preview-stage">
                <iframe
                  srcdoc={themedPreview(previewHtml())}
                  title={title()}
                  sandbox={previewSandbox("static")}
                  class="ui-preview-frame"
                />
              </div>
            </Show>

            <Show when={viewMode() === "source"}>
              <div class="ui-preview-source">
                <div class="ui-preview-source-head">
                  <span class="ui-preview-path">
                    Page source
                  </span>
                  <button type="button" class="pill-action-btn" onClick={handleCopySource}>
                    {copied() ? "✓ Copied" : copyFailed() ? "Copy failed" : "Copy Source"}
                  </button>
                </div>
                <pre class="ui-preview-code">
                  <code>{htmlContent()}</code>
                </pre>
              </div>
            </Show>
          </Show>
        </Show>
      </div>
    </div>
  );
}

/**
 * `media` primitive, covering the former inline `link.preview` / `media.image`
 * / `media.video` / `media.audio` registry cases. `kind` selects the element.
 *
 * Workspace bytes are loaded through the authenticated reader and held as
 * object URLs. Self-contained supported media can render directly; remote
 * sources stay explicit links. Link thumbnails use the same media allowlist.
 */
function renderMedia(node: AdaptiveRenderNode, _surface: RenderSurface) {
  const kind = (str(node.props, "kind") ?? "link").toLowerCase();
  const alt = () => str(node.props, "alt") ?? "";

  if (kind === "image" || kind === "video" || kind === "audio") {
    const pageReader = useContext(PresentationPageResourceContext);
    const session = pageReader ? undefined : useContext(PresentationSessionContext) ?? activeId() ?? undefined;
    const reader = pageReader ?? api.workspaceReader(session);
    const origin = createMemo(() => mediaSource(str(node.props, "source") ?? ""));
    const [fileUrl, setFileUrl] = createSignal<string>();
    const [fileError, setFileError] = createSignal<string>();
    let request = 0;
    let held: string | undefined;
    const release = () => {
      if (held) URL.revokeObjectURL(held);
      held = undefined;
      setFileUrl(undefined);
    };
    createEffect(() => {
      const from = origin();
      const generation = ++request;
      release();
      setFileError(undefined);
      if (from.kind !== "workspace") return;
      reader
        .readFileRaw(from.path)
        .then((url) => {
          if (generation !== request) return URL.revokeObjectURL(url);
          held = url;
          setFileUrl(url);
        })
        .catch((e) => {
          if (generation === request) setFileError(e instanceof Error ? e.message : String(e));
        });
    });
    onCleanup(() => {
      request += 1;
      release();
    });
    const src = () => {
      const from = origin();
      if (from.kind === "web") return safeUrl(from.url, true) ? from.url : undefined;
      return from.kind === "workspace" ? fileUrl() : undefined;
    };
    const remoteLink = () => {
      const from = origin();
      return from.kind === "web" && safeUrl(from.url) ? from.url : undefined;
    };
    const unavailable = () =>
      fileError() ?? (origin().kind === "web" && !uiPreferences.externalMedia ? "Images from the web are off in Settings." : undefined);
    return (
      <Show when={src()} fallback={<Show when={remoteLink()} fallback={<Show when={unavailable()}>{(why) => <figure class="rich-media"><figcaption>{alt() || "Media"}: {why()}</figcaption></figure>}</Show>}>{(url) => <a class="rich-link-card" href={url()} target="_blank" rel="noreferrer noopener">{alt() || "Open external media"}</a>}</Show>}>
        {(url) =>
          kind === "image" ? (
            <figure class="rich-media">
              <img src={url()} alt={alt()} />
              <Show when={alt()}>
                <figcaption>{alt()}</figcaption>
              </Show>
            </figure>
          ) : kind === "video" ? (
            <video class="rich-video" src={url()} controls preload="metadata" autoplay={uiPreferences.autoplayMedia} aria-label={alt() || "Video"} />
          ) : (
            <audio class="rich-audio" src={url()} controls preload="metadata" aria-label={alt() || "Audio"} />
          )
        }
      </Show>
    );
  }

  const url = () => str(node.props, "url") ?? "";
  const imageUrl = () => str(node.props, "image_url");
  return (
    <a class="rich-link-card" href={safeUrl(url()) ? url() : undefined} target="_blank" rel="noreferrer noopener">
      <Show when={uiPreferences.externalMedia && imageUrl() !== undefined && safeUrl(imageUrl()!, true)}>
        <img src={imageUrl()!} alt="" loading="lazy" />
      </Show>
      <span>
        <strong>{str(node.props, "title") ?? url()}</strong>
        <small>{str(node.props, "description") ?? str(node.props, "site_name") ?? url()}</small>
      </span>
    </a>
  );
}

