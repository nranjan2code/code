import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import {
  canvasArtifact,
  canvasMode,
  canvasDevice,
  setCanvasDevice,
  canvasOpen,
  closeArtifactCanvas,
  toggleCanvasMode,
  activeId,
  openCandidateReview,
  type ActiveComponentPreview,
} from "../store";
import * as api from "../api";
import Icon from "./Icon";
import { sandboxedSrcdoc } from "../safeUrl";
import { artifactPreviewHtml } from "../artifactPreview";
import { activate } from "../App";

export type ArtifactDisplayType = "html" | "pdf" | "image" | "code" | "server";

/**
 * ArtifactCanvas — immersive overlay preview for showcaseable artifacts.
 *
 * Replaces the dock-panel preview with a full-height canvas that slides in
 * from the right. In "split" mode the chat stays visible and interactive
 * side-by-side; in "focused" mode the canvas takes the full viewport width.
 *
 * Supports polyglot artifacts:
 * - HTML: sandboxed iframe with CSP
 * - Dev servers: live localhost port with automatic process lifecycle management
 * - PDF: native browser viewer via authenticated blob stream
 * - Images (png/jpg/webp/svg): centered responsive image inspector
 * - Code/Text: formatted source with copy
 *
 * Design-61 compliant: never auto-opens. Only appears on explicit user action.
 */
export default function ArtifactCanvas() {
  const [html, setHtml] = createSignal("");
  const [mediaUrl, setMediaUrl] = createSignal<string | null>(null);
  const [displayType, setDisplayType] = createSignal<ArtifactDisplayType>("html");
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [viewMode, setViewMode] = createSignal<"preview" | "source">("preview");
  const [rawText, setRawText] = createSignal("");
  const [copied, setCopied] = createSignal(false);
  const [activeServerPort, setActiveServerPort] = createSignal<number | null>(null);
  const [reloadKey, setReloadKey] = createSignal(0);
  const [isClosing, setIsClosing] = createSignal(false);
  const [feedback, setFeedback] = createSignal("");
  const [feedbackState, setFeedbackState] = createSignal<"idle" | "sending" | "sent" | "saved_only" | "error">("idle");
  const [commentLineStart, setCommentLineStart] = createSignal("");
  const [commentLineEnd, setCommentLineEnd] = createSignal("");
  const [candidateComments, setCandidateComments] = createSignal<api.SandboxCandidateComment[]>([]);
  const [candidateVersion, setCandidateVersion] = createSignal<number | null>(null);
  const commentLineInvalid = () => {
    const start = Number.parseInt(commentLineStart(), 10);
    const end = Number.parseInt(commentLineEnd(), 10);
    return commentLineEnd() !== "" && (!Number.isFinite(start) || !Number.isFinite(end) || start < 1 || end < start);
  };

  let request = 0;
  let startedServerName: string | null = null;
  let allocatedMediaUrl: string | null = null;
  let closeTimeout: ReturnType<typeof setTimeout> | undefined;

  createEffect(() => {
    const artifact = canvasArtifact();
    setFeedback("");
    setFeedbackState("idle");
    setCommentLineStart("");
    setCommentLineEnd("");
    setCandidateComments([]);
    setCandidateVersion(null);
    if (artifact?.candidateId && artifact.sessionId) {
      let disposed = false;
      void Promise.allSettled([
        api.listSandboxCandidateComments(artifact.sessionId, artifact.candidateId),
        api.listSessionSandboxRecords(artifact.sessionId),
      ]).then(([commentsResult, recordsResult]) => {
        if (disposed) return;
        if (commentsResult.status === "fulfilled") setCandidateComments(commentsResult.value.comments);
        if (recordsResult.status === "fulfilled") {
          const versions = recordsResult.value.records.filter((record) => record.kind === "Candidate" && (!artifact.executionId || record.record.execution_id === artifact.executionId));
          const index = versions.findIndex((record) => record.kind === "Candidate" && record.record.candidate.candidate_id === artifact.candidateId);
          if (index >= 0) setCandidateVersion(index + 1);
        }
      });
      onCleanup(() => { disposed = true; });
    }
  });

  createEffect(() => {
    const artifact = canvasArtifact();
    if (!canvasOpen() || !artifact?.candidateId || !artifact.sessionId) return;
    const sessionId = artifact.sessionId;
    const candidateId = artifact.candidateId;
    let disposed = false;
    const updates = api.openCoworkingUpdates(sessionId);
    updates.addEventListener("refresh", () => {
      void api.listSandboxCandidateComments(sessionId, candidateId)
        .then(({ comments }) => {
          if (!disposed && canvasArtifact()?.candidateId === candidateId) setCandidateComments(comments);
        })
        .catch(() => { /* Preserve the visible comment history while offline. */ });
    });
    onCleanup(() => { disposed = true; updates.close(); });
  });

  const sendRevision = async () => {
    const artifact = canvasArtifact();
    const sessionId = artifact?.sessionId ?? activeId();
    const note = feedback().trim();
    if (!sessionId || !artifact || !note || feedbackState() === "sending") return;
    setFeedbackState("sending");
    let commentSaved = false;
    const subject = artifact.artifactPath || artifact.title;
    try {
      if (artifact.candidateId) {
        const lineStart = Number.parseInt(commentLineStart(), 10);
        const lineEnd = Number.parseInt(commentLineEnd(), 10);
        const saved = await api.commentOnSandboxCandidate(sessionId, artifact.candidateId, note, {
          path: artifact.artifactPath || undefined,
          lineStart: Number.isFinite(lineStart) && lineStart > 0 ? lineStart : undefined,
          lineEnd: Number.isFinite(lineEnd) && lineEnd > 0 ? lineEnd : undefined,
        });
        commentSaved = true;
        setFeedback("");
        void api.listSandboxCandidateComments(sessionId, artifact.candidateId)
          .then(({ comments }) => setCandidateComments(comments))
          .catch(() => { /* The accepted comment remains durable. */ });
        await api.requestRevisionFromCandidateComment(sessionId, artifact.candidateId, saved.comment_id);
      } else {
        const result = artifact.resultId ? ` from result ${artifact.resultId}` : "";
        await api.steer(sessionId, `Please revise the draft ${JSON.stringify(subject)}${result}. Feedback: ${note}`);
      }
      setFeedback("");
      setFeedbackState("sent");
    } catch {
      setFeedbackState(commentSaved ? "saved_only" : "error");
    }
  };

  // Cleanup helper for dev servers started specifically by the canvas
  const cleanupServer = () => {
    if (startedServerName) {
      const sid = canvasArtifact()?.sessionId ?? activeId();
      if (sid) void api.stopLaunch(sid, startedServerName, canvasArtifact()?.candidateId);
      startedServerName = null;
    }
  };

  // Cleanup helper for blob URLs created for PDF/image previews
  const cleanupMedia = () => {
    if (allocatedMediaUrl) {
      URL.revokeObjectURL(allocatedMediaUrl);
      allocatedMediaUrl = null;
    }
    setMediaUrl(null);
  };

  onCleanup(() => {
    request += 1;
    if (closeTimeout) clearTimeout(closeTimeout);
    cleanupServer();
    cleanupMedia();
  });

  const detectType = (artifact: ActiveComponentPreview): ArtifactDisplayType => {
    if (artifact.serverName || artifact.serverUrl) return "server";
    const p = (artifact.artifactPath || "").toLowerCase();
    if (p.endsWith(".pdf")) return "pdf";
    if (
      p.endsWith(".png") ||
      p.endsWith(".jpg") ||
      p.endsWith(".jpeg") ||
      p.endsWith(".gif") ||
      p.endsWith(".webp") ||
      p.endsWith(".svg") ||
      p.endsWith(".ico") ||
      p.endsWith(".bmp")
    ) {
      return "image";
    }
    if (p.endsWith(".html") || p.endsWith(".htm") || p.endsWith(".xhtml")) {
      return "html";
    }
    // If inline html is provided without a conflicting non-html file path, treat as html
    if (artifact.html && !p) {
      return "html";
    }
    // Any file with a path (source code, config, logs, etc.) is rendered in the code viewer
    if (p) {
      return "code";
    }
    return "html";
  };

  const loadContent = async (artifact: ActiveComponentPreview) => {
    const generation = ++request;
    setLoading(true);
    setError(null);
    cleanupMedia();

    const kind = detectType(artifact);
    setDisplayType(kind);
    // Automatically configure default viewMode: code artifacts default to source; web/media default to preview
    if (kind === "code") {
      setViewMode("source");
    } else {
      setViewMode("preview");
    }

    try {
      if (artifact.candidateId && !artifact.sessionId) throw new Error("Saved draft has no owning conversation.");
      const readText = () => artifact.candidateId && artifact.sessionId
        ? api.readSandboxCandidateFile(artifact.sessionId, artifact.candidateId, artifact.artifactPath)
        : api.readFile(artifact.artifactPath);
      const readRaw = () => artifact.candidateId && artifact.sessionId
        ? api.readSandboxCandidateFileRaw(artifact.sessionId, artifact.candidateId, artifact.artifactPath)
        : api.readFileRaw(artifact.artifactPath);
      // 1. Dev-server handling: if serverName is provided, ensure it is running
      if (artifact.serverName) {
        const sid = artifact.sessionId ?? activeId();
        if (sid) {
          if (startedServerName && startedServerName !== artifact.serverName) {
            cleanupServer();
          }
          const res = await api.startLaunch(sid, artifact.serverName, artifact.candidateId);
          if (res.error && !res.error.toLowerCase().includes("already running")) {
            throw new Error(res.error);
          }
          if (generation !== request) return;
          startedServerName = artifact.serverName;
          const launchState = await api.getLaunch(sid, artifact.candidateId);
          if (generation !== request) return;
          const srv = launchState.servers.find((s) => s.name === artifact.serverName);
          if (srv?.port) {
            setActiveServerPort(srv.port);
            setRawText(`<!-- Running dev server: ${artifact.serverName} at http://127.0.0.1:${srv.port} -->`);
            setHtml("");
            return;
          }
          throw new Error(`Dev server "${artifact.serverName}" started, but did not report a listening port.`);
        }
      } else if (artifact.serverUrl) {
        if (startedServerName) cleanupServer();
        setActiveServerPort(null);
        if (generation !== request) return;
        setRawText(`<!-- External server: ${artifact.serverUrl} -->`);
        setHtml("");
        return;
      }

      // If switching away from dev server, shut down previously started server
      if (startedServerName) cleanupServer();
      setActiveServerPort(null);

      // 2. PDF Document handling: load raw bytes into a blob URL
      if (kind === "pdf") {
        if (!artifact.artifactPath) throw new Error("PDF artifact path is missing.");
        const url = await readRaw();
        allocatedMediaUrl = url;
        if (generation !== request) {
          URL.revokeObjectURL(url);
          return;
        }
        setMediaUrl(url);
        setRawText(`[PDF Document: ${artifact.artifactPath}]`);
        return;
      }

      // 3. Image handling: load raw bytes into an image blob URL
      if (kind === "image") {
        if (!artifact.artifactPath) throw new Error("Image artifact path is missing.");
        const url = await readRaw();
        allocatedMediaUrl = url;
        if (generation !== request) {
          URL.revokeObjectURL(url);
          return;
        }
        setMediaUrl(url);
        setRawText(`[Image: ${artifact.artifactPath}]`);
        return;
      }

      // 4. Source / Text-only document handling
      if (kind === "code") {
        let text = !artifact.candidateId && artifact.html && artifact.html.trim().length > 0 ? artifact.html : undefined;
        if (artifact.artifactPath) {
          try {
            const fileRes = await readText();
            if (fileRes.content !== undefined) text = fileRes.content;
          } catch (error) {
            if (artifact.candidateId) throw error;
            // Keep inline content if disk file was not found
          }
        }
        if (text === undefined) {
          throw new Error(
            artifact.artifactPath
              ? `Document "${artifact.artifactPath}" has not been created on disk yet.`
              : "Document content is unavailable."
          );
        }
        if (generation !== request) return;
        setRawText(text);
        setViewMode("source");
        return;
      }

      // 5. Static / HTML preview handling
      let content = !artifact.candidateId && artifact.html && artifact.html.trim().length > 0 ? artifact.html : undefined;
      if (artifact.artifactPath) {
        try {
          const fileRes = await readText();
          if (fileRes.content !== undefined) {
            content = fileRes.content;
          }
        } catch (error) {
          if (artifact.candidateId) throw error;
          // Keep inline content if disk read fails (e.g. file referenced before save)
        }
      }

      if (content === undefined) {
        throw new Error(
          artifact.artifactPath
            ? `File "${artifact.artifactPath}" has not been created on disk yet.`
            : "Preview content is unavailable."
        );
      }

      let prepared: string;
      try {
        prepared = artifact.artifactPath
          ? await artifactPreviewHtml(
              artifact.artifactPath,
              content,
              artifact.candidateId ? "'none'" : artifact.connectSrc,
              artifact.candidateId && artifact.sessionId
                ? {
                    readFile: (path) => api.readSandboxCandidateFile(artifact.sessionId!, artifact.candidateId!, path),
                    readFileRaw: (path) => api.readSandboxCandidateFileRaw(artifact.sessionId!, artifact.candidateId!, path),
                  }
                : api,
            )
          : sandboxedSrcdoc(content, artifact.connectSrc ?? "'none'");
      } catch {
        // If relative asset resolution fails, fall back to pure sandboxed srcdoc
        prepared = sandboxedSrcdoc(content, artifact.candidateId ? "'none'" : artifact.connectSrc ?? "'none'");
      }

      if (generation !== request) return;
      setRawText(content);
      setHtml(prepared);
    } catch (e) {
      if (generation === request)
        setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (generation === request) setLoading(false);
    }
  };

  // Load content ONLY when the canvas artifact changes (decoupled from layout/mode changes)
  createEffect(() => {
    const artifact = canvasArtifact();
    if (!artifact) {
      setHtml("");
      setRawText("");
      setError(null);
      setViewMode("preview");
      setActiveServerPort(null);
      cleanupServer();
      cleanupMedia();
      return;
    }
    // Cancel any in-flight closing transition so immediate reopening doesn't get dismissed
    if (closeTimeout) {
      clearTimeout(closeTimeout);
      closeTimeout = undefined;
    }
    setIsClosing(false);
    void loadContent(artifact);
  });

  const reload = () => {
    setReloadKey((k) => k + 1);
    const artifact = canvasArtifact();
    if (artifact) void loadContent(artifact);
  };

  const handleClose = () => {
    if (isClosing()) return;
    setIsClosing(true);
    closeTimeout = setTimeout(() => {
      closeArtifactCanvas();
      setIsClosing(false);
      closeTimeout = undefined;
    }, 220);
  };

  const popout = () => {
    if (activeServerPort()) {
      window.open(`http://127.0.0.1:${activeServerPort()}`, "_blank", "noopener,noreferrer");
      return;
    }
    const currentArtifact = canvasArtifact();
    if (currentArtifact?.serverUrl) {
      window.open(currentArtifact.serverUrl, "_blank", "noopener,noreferrer");
      return;
    }
    if (mediaUrl()) {
      window.open(mediaUrl()!, "_blank", "noopener,noreferrer");
      return;
    }
    const h = html();
    if (h) {
      const blob = new Blob([h], { type: "text/html" });
      const u = URL.createObjectURL(blob);
      window.open(u, "_blank", "noopener,noreferrer");
      setTimeout(() => URL.revokeObjectURL(u), 60000);
      return;
    }
    const t = rawText();
    if (t) {
      const mime = displayType() === "code" ? "text/plain" : "text/html";
      const blob = new Blob([t], { type: mime });
      const u = URL.createObjectURL(blob);
      window.open(u, "_blank", "noopener,noreferrer");
      setTimeout(() => URL.revokeObjectURL(u), 60000);
      return;
    }
  };

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(rawText());
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard may not be available in all contexts
    }
  };

  // Close on Escape key when canvas is open
  createEffect(() => {
    if (!canvasOpen()) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        handleClose();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    onCleanup(() => document.removeEventListener("keydown", onKeyDown));
  });

  const title = () =>
    canvasArtifact()?.title || "Artifact Preview";
  const path = () =>
    canvasArtifact()?.artifactPath || "";
  const mode = () => canvasMode();
  const device = () => canvasDevice();

  const serverSrc = () => {
    const port = activeServerPort();
    const base = port ? `http://127.0.0.1:${port}` : (canvasArtifact()?.serverUrl ?? "");
    if (!base) return undefined;
    const separator = base.includes("?") ? "&" : "?";
    const artifact = canvasArtifact();
    const candidatePath = artifact?.candidateId && artifact.artifactPath
      ? `/${artifact.artifactPath.split("/").map(encodeURIComponent).join("/")}`
      : "";
    return `${base}${candidatePath}${separator}_k=${reloadKey()}`;
  };
  const sourceLines = () => rawText().split("\n");
  const selectedLine = (line: number) => {
    const start = Number.parseInt(commentLineStart(), 10);
    const end = Number.parseInt(commentLineEnd(), 10);
    if (!Number.isFinite(start)) return false;
    return line >= start && line <= (Number.isFinite(end) ? end : start);
  };
  const selectSourceLine = (line: number, extend: boolean) => {
    const start = Number.parseInt(commentLineStart(), 10);
    if (extend && Number.isFinite(start)) {
      setCommentLineStart(String(Math.min(start, line)));
      setCommentLineEnd(String(Math.max(start, line)));
    } else {
      setCommentLineStart(String(line));
      setCommentLineEnd("");
    }
  };
  const commentsForArtifact = () => candidateComments().filter((comment) => !comment.path || comment.path === path());
  const returnToReview = () => {
    const artifact = canvasArtifact();
    const executionId = artifact?.executionId;
    if (!executionId) return;
    closeArtifactCanvas();
    openCandidateReview(executionId, artifact?.sessionId, artifact?.candidateId);
  };
  const returnToConversation = () => {
    const sessionId = canvasArtifact()?.sessionId;
    closeArtifactCanvas();
    if (sessionId && sessionId !== activeId()) void activate(sessionId);
  };

  return (
    <Show when={canvasOpen()}>
      {/* Backdrop: pointer-events: none in split mode to keep chat completely interactive; active in focused mode */}
      <div
        class="artifact-canvas-backdrop"
        classList={{
          focused: mode() === "focused",
          "canvas-closing": isClosing(),
        }}
        onClick={(e) => {
          if (e.target === e.currentTarget && mode() === "focused") handleClose();
        }}
        role="presentation"
      />

      {/* Canvas panel */}
      <div
        class="artifact-canvas"
        classList={{
          "canvas-split": mode() === "split",
          "canvas-focused": mode() === "focused",
          "canvas-closing": isClosing(),
        }}
        role="dialog"
        aria-label={`Artifact preview: ${title()}`}
        aria-modal={mode() === "focused" ? "true" : "false"}
      >
        {/* Title bar */}
        <header class="artifact-canvas-header">
          <div class="artifact-canvas-title-group">
            <span class="artifact-canvas-badge">
              {displayType() === "pdf" ? "PDF" : displayType() === "image" ? "Image" : displayType() === "server" ? "Dev Server" : "Preview"}
            </span>
            <strong class="artifact-canvas-title">{title()}</strong>
            <Show when={path()}>
              <span class="artifact-canvas-path">{path()}</span>
            </Show>
            <Show when={canvasArtifact()?.resultId}>{(resultId) =>
              <button type="button" class="artifact-canvas-result" title={resultId()} onClick={returnToConversation}>Back to conversation result</button>
            }</Show>
            <Show when={canvasArtifact()?.candidateId}>{(candidateId) =>
              <span class="artifact-canvas-result" title={candidateId()}>Saved draft{candidateVersion() ? ` · Version ${candidateVersion()}` : ""}</span>
            }</Show>
          </div>

          <div class="artifact-canvas-controls">
            {/* View Mode Segmented Controls (for HTML and code artifacts) */}
            <Show when={displayType() === "html" || displayType() === "code"}>
              <div class="artifact-canvas-segmented">
                <button
                  type="button"
                  class="artifact-canvas-seg-btn"
                  classList={{ active: viewMode() === "preview" }}
                  onClick={() => setViewMode("preview")}
                  title="View interactive preview"
                >
                  Preview
                </button>
                <button
                  type="button"
                  class="artifact-canvas-seg-btn"
                  classList={{ active: viewMode() === "source" }}
                  onClick={() => setViewMode("source")}
                  title="View source markup"
                >
                  Code
                </button>
              </div>
            </Show>

            {/* Responsive Viewport Switcher (HTML / Server preview mode only) */}
            <Show when={viewMode() === "preview" && (displayType() === "html" || displayType() === "server")}>
              <div class="artifact-canvas-device-group">
                <button
                  type="button"
                  class="artifact-canvas-device-btn"
                  classList={{ active: device() === "desktop" }}
                  onClick={() => setCanvasDevice("desktop")}
                  title="Desktop (100%)"
                  aria-label="Desktop viewport"
                >
                  100%
                </button>
                <button
                  type="button"
                  class="artifact-canvas-device-btn"
                  classList={{ active: device() === "tablet" }}
                  onClick={() => setCanvasDevice("tablet")}
                  title="Tablet (768px)"
                  aria-label="Tablet viewport"
                >
                  768px
                </button>
                <button
                  type="button"
                  class="artifact-canvas-device-btn"
                  classList={{ active: device() === "mobile" }}
                  onClick={() => setCanvasDevice("mobile")}
                  title="Mobile (375px)"
                  aria-label="Mobile viewport"
                >
                  375px
                </button>
              </div>
            </Show>

            {/* Action Buttons */}
            <button
              type="button"
              class="artifact-canvas-btn"
              onClick={reload}
              title="Reload preview"
            >
              <Icon name="sync" size={14} />
            </button>
            <button
              type="button"
              class="artifact-canvas-btn"
              onClick={toggleCanvasMode}
              title={
                mode() === "split"
                  ? "Expand to full width"
                  : "Shrink to split view"
              }
            >
              <Icon name={mode() === "split" ? "layers" : "restore"} size={14} />
            </button>
            <button
              type="button"
              class="artifact-canvas-btn"
              onClick={popout}
              title="Open in new window"
            >
              <Icon name="preview" size={14} />
            </button>
            <Show when={canvasArtifact()?.candidateId && canvasArtifact()?.executionId}>
              <button type="button" class="artifact-canvas-btn" onClick={returnToReview} title="Return to candidate review">
                <Icon name="diff" size={14} /> Review draft
              </button>
            </Show>
            <button
              type="button"
              class="artifact-canvas-close"
              onClick={handleClose}
              title="Close canvas (Esc)"
              aria-label="Close artifact canvas"
            >
              <Icon name="close" size={16} />
            </button>
          </div>
        </header>

        {/* Content Body */}
        <div class="artifact-canvas-body">
          <Show when={loading()}>
            <div class="artifact-canvas-loading">
              <div class="artifact-canvas-spinner" />
              <span>Preparing preview…</span>
            </div>
          </Show>

          <Show when={error()}>
            <div class="artifact-canvas-error">
              <Icon name="warning" size={16} />
              <span>{error()}</span>
              <button
                type="button"
                class="artifact-canvas-btn"
                onClick={reload}
              >
                Retry
              </button>
            </div>
          </Show>

          <Show when={!loading() && !error()}>
            {/* View Mode: Preview */}
            <Show when={viewMode() === "preview"}>
              {/* Image Preview */}
              <Show when={displayType() === "image" && mediaUrl()}>
                <div class="artifact-canvas-image-container">
                  <img
                    class="artifact-canvas-image"
                    src={mediaUrl()!}
                    alt={title()}
                  />
                </div>
              </Show>

              {/* PDF Preview */}
              <Show when={displayType() === "pdf" && mediaUrl()}>
                <iframe
                  class="artifact-canvas-pdf-frame"
                  src={mediaUrl()!}
                  title={title()}
                />
              </Show>

              {/* HTML / Dev-Server Preview */}
              <Show when={displayType() === "html" || displayType() === "server"}>
                <div
                  class="artifact-canvas-viewport"
                  classList={{
                    "device-desktop": device() === "desktop",
                    "device-tablet": device() === "tablet",
                    "device-mobile": device() === "mobile",
                  }}
                >
                  <div class="artifact-canvas-frame-container">
                    <iframe
                      class="artifact-canvas-frame"
                      src={serverSrc()}
                      srcdoc={serverSrc() ? undefined : html()}
                      title={title()}
                      sandbox={
                        canvasArtifact()?.sandbox ??
                        (activeServerPort() || canvasArtifact()?.serverUrl
                          ? "allow-scripts allow-same-origin allow-forms allow-popups"
                          : "allow-scripts allow-forms")
                      }
                    />
                  </div>
                </div>
              </Show>
            </Show>

            {/* View Mode: Source */}
            <Show when={viewMode() === "source"}>
              <div class="artifact-canvas-source">
                <div class="artifact-canvas-source-head">
                  <span class="artifact-canvas-source-path">
                    {path() || "inline markup"}
                  </span>
                  <button
                    type="button"
                    class="artifact-canvas-btn"
                    onClick={handleCopy}
                  >
                    <Icon name="copy" size={13} />
                    {copied() ? "Copied!" : "Copy"}
                  </button>
                </div>
                <div class="artifact-canvas-code" role="list" aria-label="Source lines">
                  <For each={sourceLines()}>{(line, index) => {
                    const lineNumber = index() + 1;
                    return <div class="artifact-canvas-code-line" classList={{ selected: selectedLine(lineNumber) }} role="listitem">
                      <button type="button" class="artifact-canvas-line-number" aria-label={`Select line ${lineNumber}`} aria-pressed={selectedLine(lineNumber)} onClick={(event) => selectSourceLine(lineNumber, event.shiftKey)}>{lineNumber}</button>
                      <code>{line || " "}</code>
                    </div>;
                  }}</For>
                </div>
              </div>
            </Show>
          </Show>
        </div>

        <section class="artifact-canvas-feedback" aria-label="Review this draft">
          <div class="artifact-canvas-feedback-intro">
            <strong>Work on this together</strong>
            <span>Tell the Agent what to change in this draft.</span>
          </div>
          <div class="artifact-canvas-feedback-compose">
            <Show when={canvasArtifact()?.candidateId && viewMode() === "source"}>
              <div class="artifact-canvas-line-anchor" aria-label="Comment location">
                <label for="canvas-comment-line-start">Line</label>
                <input id="canvas-comment-line-start" type="number" min="1" inputmode="numeric" value={commentLineStart()} onInput={(event) => setCommentLineStart(event.currentTarget.value)} placeholder="Start" />
                <span>to</span>
                <input type="number" min={commentLineStart() || "1"} inputmode="numeric" disabled={!commentLineStart()} value={commentLineEnd()} onInput={(event) => setCommentLineEnd(event.currentTarget.value)} placeholder="End" aria-label="End line" />
                <button type="button" class="artifact-canvas-btn" onClick={() => { setCommentLineStart(""); setCommentLineEnd(""); }}>Whole file</button>
              </div>
              <Show when={commentLineInvalid()}><small role="alert">End line must be on or after the start line.</small></Show>
            </Show>
            <textarea
              rows={2}
              value={feedback()}
              onInput={(event) => { setFeedback(event.currentTarget.value); setFeedbackState("idle"); }}
              onKeyDown={(event) => {
                if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
                  event.preventDefault();
                  void sendRevision();
                }
              }}
              placeholder="What should change?"
              aria-label="Feedback for this draft"
            />
            <button type="button" disabled={!feedback().trim() || feedbackState() === "sending" || commentLineInvalid()} onClick={() => void sendRevision()}>
              {feedbackState() === "sending" ? "Sending…" : "Ask for revision"}
            </button>
          </div>
          <Show when={feedbackState() === "sent"}><small role="status">Sent to the Agent conversation.</small></Show>
          <Show when={feedbackState() === "saved_only"}><small role="status">Comment saved. Open Review to ask the Agent to address it.</small></Show>
          <Show when={feedbackState() === "error"}><small role="alert">Could not send. Your feedback is still here to retry.</small></Show>
          <Show when={commentsForArtifact().length > 0}>
            <div class="artifact-canvas-comments" aria-label="Comments on this saved draft">
              <For each={commentsForArtifact()}>{(comment) =>
                <article>
                  <div><strong>{comment.actor_id === "operator" ? "You" : comment.actor_name ?? comment.actor_id}</strong><span>{comment.path}{comment.line_start ? ` · line ${comment.line_start}${comment.line_end && comment.line_end !== comment.line_start ? `–${comment.line_end}` : ""}` : ""}</span></div>
                  <p>{comment.text}</p>
                </article>
              }</For>
            </div>
          </Show>
        </section>

        {/* Security / Server Status Footer */}
        <footer class="artifact-canvas-footer">
          <span class="artifact-canvas-security">
            <Icon name="shield" size={12} />
            <Show
              when={activeServerPort() || canvasArtifact()?.serverUrl}
              fallback={
                <>
                  Sandboxed · net: {canvasArtifact()?.connectSrc ?? "blocked"}
                </>
              }
            >
              Dev server: {activeServerPort() ? `http://127.0.0.1:${activeServerPort()}` : canvasArtifact()?.serverUrl}
            </Show>
          </span>
        </footer>
      </div>
    </Show>
  );
}
