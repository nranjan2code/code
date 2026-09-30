import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import {
  canvasSubject,
  canvasMode,
  canvasDevice,
  setCanvasDevice,
  canvasOpen,
  closeArtifactCanvas,
  toggleCanvasMode,
  activeId,
  openCandidateReview,
  technicalDetails,
  coworkingPresence,
} from "../store";
import * as api from "../api";
import { watchCoworking } from "../streamHub";
import Icon from "./Icon";
import { previewSandbox, sandboxedSrcdoc } from "../safeUrl";
import { artifactPreviewHtml, subjectReader } from "../artifactPreview";
import { parseDelimitedPreview, type DelimitedPreview } from "../delimitedPreview";
import { activate, sendPrompt } from "../App";
import OfficeWorkspacePane from "./OfficeWorkspacePane";
import { pdfAnchorPage } from "../officeFiles";
import {
  displayType as subjectDisplayType,
  subjectCandidateId,
  subjectExecutionId,
  subjectPath,
  subjectSessionId,
  type ArtifactDisplayType,
  type CanvasSubject,
} from "../canvasSubject";

/**
 * ArtifactCanvas — immersive overlay preview for showcaseable artifacts.
 *
 * Replaces the dock-panel preview with a full-height canvas that slides in
 * from the right. In "split" mode the chat stays visible and interactive
 * side-by-side; in "focused" mode the canvas takes the full viewport width.
 *
 * It shows a `CanvasSubject` (../canvasSubject.ts): what is open is named by
 * identity, and each kind is read through the one route its identity names —
 * a workspace file, a run's scratch file, a saved version of a draft, markup
 * from the conversation, or a dev server. There is no fallback between them.
 *
 * Supports polyglot artifacts:
 * - HTML: sandboxed iframe with CSP
 * - Dev servers: live localhost port with automatic process lifecycle management
 * - PDF: native browser viewer via authenticated blob stream
 * - Images (png/jpg/webp/svg): centered responsive image inspector
 * - CSV/TSV: bounded table preview with source available
 * - Word/Excel/PowerPoint/Visio: the Office views, drawn from the server's
 *   projection (docs/design/72, P4)
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
  const [previewWarning, setPreviewWarning] = createSignal<string | null>(null);
  const [preparationRequired, setPreparationRequired] = createSignal(false);
  const [viewMode, setViewMode] = createSignal<"preview" | "source">("preview");
  /** Small screens open the readable text view; the original pages stay available. */
  const [pdfText, setPdfText] = createSignal(false);
  const [rawText, setRawText] = createSignal("");
  const [tablePreview, setTablePreview] = createSignal<DelimitedPreview | null>(null);
  const [copied, setCopied] = createSignal(false);
  const [activeServerPort, setActiveServerPort] = createSignal<number | null>(null);
  const [reloadKey, setReloadKey] = createSignal(0);
  const [isClosing, setIsClosing] = createSignal(false);
  const [feedback, setFeedback] = createSignal("");
  const [showFeedback, setShowFeedback] = createSignal(true);
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

  /** The saved version being viewed, when the subject is one. */
  const draft = () => {
    const subject = canvasSubject();
    return subject?.kind === "draft_file" ? subject : undefined;
  };
  const candidateId = () => { const subject = canvasSubject(); return subject ? subjectCandidateId(subject) : undefined; };
  const executionId = () => { const subject = canvasSubject(); return subject ? subjectExecutionId(subject) : undefined; };
  const ownerSessionId = () => { const subject = canvasSubject(); return subject ? subjectSessionId(subject) : undefined; };
  const title = () => canvasSubject()?.title || "Artifact Preview";
  const path = () => { const subject = canvasSubject(); return subject ? subjectPath(subject) : ""; };

  let request = 0;
  /** The dev server this Canvas started, with the identity it was started under. */
  let startedServer: { sessionId: string; name: string; candidateId?: string } | null = null;
  let allocatedMediaUrl: string | null = null;
  let closeTimeout: ReturnType<typeof setTimeout> | undefined;

  createEffect(() => {
    const subject = canvasSubject();
    const file = subject ? subjectPath(subject) : "";
    setPdfText(/\.pdf$/i.test(file) && window.matchMedia("(max-width: 1100px)").matches);
    setFeedback("");
    setShowFeedback(!/\.(docx|xlsx|pptx|pdf)$/i.test(file));
    setFeedbackState("idle");
    setCommentLineStart("");
    setCommentLineEnd("");
    setCandidateComments([]);
    setCandidateVersion(null);
    setPreparationRequired(false);
    const version = subject?.kind === "draft_file" ? subject : undefined;
    if (version) {
      let disposed = false;
      void Promise.allSettled([
        api.listSandboxCandidateComments(version.sessionId, version.candidateId),
        api.listSessionSandboxRecords(version.sessionId),
      ]).then(([commentsResult, recordsResult]) => {
        if (disposed) return;
        if (commentsResult.status === "fulfilled") setCandidateComments(commentsResult.value.comments);
        if (recordsResult.status === "fulfilled") {
          const versions = recordsResult.value.records.filter((record) => record.kind === "Candidate" && (!version.executionId || record.record.execution_id === version.executionId));
          const index = versions.findIndex((record) => record.kind === "Candidate" && record.record.candidate.candidate_id === version.candidateId);
          if (index >= 0) setCandidateVersion(index + 1);
        }
      });
      onCleanup(() => { disposed = true; });
    }
  });

  createEffect(() => {
    const version = draft();
    if (!canvasOpen() || !version) return;
    const { sessionId, candidateId } = version;
    let disposed = false;
    const stop = watchCoworking(sessionId, () => {
      void api.listSandboxCandidateComments(sessionId, candidateId)
        .then(({ comments }) => {
          if (!disposed && draft()?.candidateId === candidateId) setCandidateComments(comments);
        })
        .catch(() => { /* Preserve the visible comment history while offline. */ });
    });
    onCleanup(() => { disposed = true; stop(); });
  });

  const sendRevision = async () => {
    const subject = canvasSubject();
    const sessionId = (subject && subjectSessionId(subject)) ?? activeId();
    const note = feedback().trim();
    if (!sessionId || !subject || !note || feedbackState() === "sending") return;
    setFeedbackState("sending");
    let commentSaved = false;
    const subjectLabel = path() || subject.title;
    try {
      if (subject.kind === "draft_file") {
        const lineStart = Number.parseInt(commentLineStart(), 10);
        const lineEnd = Number.parseInt(commentLineEnd(), 10);
        const saved = await api.commentOnSandboxCandidate(sessionId, subject.candidateId, note, {
          path: subject.path || undefined,
          lineStart: Number.isFinite(lineStart) && lineStart > 0 ? lineStart : undefined,
          lineEnd: Number.isFinite(lineEnd) && lineEnd > 0 ? lineEnd : undefined,
        });
        commentSaved = true;
        setFeedback("");
        void api.listSandboxCandidateComments(sessionId, subject.candidateId)
          .then(({ comments }) => setCandidateComments(comments))
          .catch(() => { /* The accepted comment remains durable. */ });
        await api.requestRevisionFromCandidateComment(sessionId, subject.candidateId, saved.comment_id);
      } else {
        const result = subject.resultId ? ` from result ${subject.resultId}` : "";
        await sendPrompt(
          `Please revise the draft ${JSON.stringify(subjectLabel)}${result}. Feedback: ${note}\nInspect the saved result and answer when the change is done; do not repeat a write when the file already contains the requested change.`,
          undefined,
          undefined,
          sessionId,
          { sessionId, resultId: subject.resultId, label: subjectLabel },
          "correction",
          true,
        );
        // A new Agent turn may raise a scoped approval. Return to the
        // conversation so its live controls and result are visible.
        closeArtifactCanvas();
      }
      setFeedback("");
      setFeedbackState("sent");
    } catch {
      setFeedbackState(commentSaved ? "saved_only" : "error");
    }
  };

  // Stops the dev server this Canvas started, under the identity it was
  // started with: by the time this runs the subject may already be another.
  const cleanupServer = () => {
    if (startedServer) {
      const { sessionId, name, candidateId } = startedServer;
      void api.stopLaunch(sessionId, name, candidateId);
      startedServer = null;
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

  const loadContent = async (subject: CanvasSubject) => {
    const generation = ++request;
    setLoading(true);
    setError(null);
    setPreviewWarning(null);
    setTablePreview(null);
    cleanupMedia();

    const kind = subjectDisplayType(subject);
    setDisplayType(kind);
    // Code opens on its source; web pages and media open on the preview.
    setViewMode(kind === "code" ? "source" : "preview");

    try {
      // 1. Dev server: make sure the configured server is running, under this subject's identity.
      if (subject.kind === "live_server") {
        const { sessionId, serverName, candidateId } = subject;
        if (startedServer && startedServer.name !== serverName) cleanupServer();
        const readiness = await api.getLaunch(sessionId, candidateId);
        const configured = readiness.servers.find((server) => server.name === serverName);
        if (!configured) throw new Error(`Dev server "${serverName}" is unavailable for this saved version.`);
        if (!configured.available && !configured.running) {
          setPreparationRequired(configured.availability === "needs_preparation");
          throw new Error(configured.unavailable_reason ?? "Preview environment is not ready.");
        }
        const res = await api.startLaunch(sessionId, serverName, candidateId);
        if (res.error && !res.error.toLowerCase().includes("already running")) {
          throw new Error(res.error);
        }
        if (generation !== request) return;
        startedServer = { sessionId, name: serverName, candidateId };
        const launchState = await api.getLaunch(sessionId, candidateId);
        if (generation !== request) return;
        const running = launchState.servers.find((server) => server.name === serverName);
        if (!running?.port) throw new Error(`Dev server "${serverName}" started, but did not report a listening port.`);
        setActiveServerPort(running.port);
        setRawText(`<!-- Running dev server: ${serverName} at http://127.0.0.1:${running.port} -->`);
        setHtml("");
        return;
      }

      // Switching away from a dev server shuts down the one this Canvas started.
      cleanupServer();
      setActiveServerPort(null);

      // Office files: OfficeView reads its own pages from the projection.
      if (kind === "office") {
        setRawText("");
        return;
      }

      const reader = subjectReader(subject);
      const file = subjectPath(subject);
      const readText = async () => {
        if (!reader || !file) throw new Error("This preview has no file to read.");
        const response = await reader.readFile(file);
        if (response.content === undefined) throw new Error(`"${file}" has no readable text.`);
        return response.content;
      };

      // 2. PDF and images: load raw bytes into a blob URL.
      if (kind === "pdf" || kind === "image") {
        if (!reader || !file) throw new Error(kind === "pdf" ? "PDF artifact path is missing." : "Image artifact path is missing.");
        const url = await reader.readFileRaw(file);
        allocatedMediaUrl = url;
        if (generation !== request) {
          URL.revokeObjectURL(url);
          return;
        }
        setMediaUrl(url);
        setRawText(kind === "pdf" ? `[PDF Document: ${file}]` : `[Image: ${file}]`);
        return;
      }

      // 3. Delimited data.
      if (kind === "table") {
        const content = await readText();
        if (generation !== request) return;
        setRawText(content);
        try {
          setTablePreview(parseDelimitedPreview(content, file.toLowerCase().endsWith(".tsv") ? "\t" : ","));
        } catch (cause) {
          setPreviewWarning(cause instanceof Error ? cause.message : String(cause));
        }
        return;
      }

      // 4. Source text.
      if (kind === "code") {
        const text = await readText();
        if (generation !== request) return;
        setRawText(text);
        setViewMode("source");
        return;
      }

      // 5. HTML: the conversation's own markup, or a file read through its route.
      const content = subject.kind === "inline" ? subject.html : await readText();
      let prepared: string;
      if (file && reader) {
        try {
          prepared = await artifactPreviewHtml(file, content, reader);
        } catch {
          prepared = sandboxedSrcdoc(content);
          if (generation === request) setPreviewWarning("Some files this page uses could not be loaded, so it may look incomplete.");
        }
      } else {
        prepared = sandboxedSrcdoc(content);
      }

      if (generation !== request) return;
      setRawText(content);
      const parsed = new DOMParser().parseFromString(content, "text/html");
      if (!parsed.body?.textContent?.trim() && !parsed.body?.children.length && !parsed.querySelector("script")) {
        setPreviewWarning("This HTML has no visible page content. Open Code to inspect it or ask for a revision.");
      }
      setHtml(prepared);
    } catch (e) {
      if (generation === request)
        setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (generation === request) setLoading(false);
    }
  };

  // Load content ONLY when the canvas subject changes (decoupled from layout/mode changes)
  createEffect(() => {
    const subject = canvasSubject();
    if (!subject) {
      setHtml("");
      setRawText("");
      setTablePreview(null);
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
    void loadContent(subject);
  });

  const reload = () => {
    setReloadKey((k) => k + 1);
    const subject = canvasSubject();
    if (subject) void loadContent(subject);
  };

  const preparePreview = async () => {
    const subject = canvasSubject();
    if (subject?.kind !== "live_server" || !subject.candidateId) return;
    setLoading(true);
    setError(null);
    try {
      await api.prepareLaunch(subject.sessionId, subject.serverName, subject.candidateId);
      setPreparationRequired(false);
      await loadContent(subject);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setLoading(false);
    }
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

  const mode = () => canvasMode();
  const device = () => canvasDevice();

  const serverSrc = () => {
    const port = activeServerPort();
    if (!port) return undefined;
    const subject = canvasSubject();
    const candidatePath = subject?.kind === "live_server" && subject.candidateId && subject.path
      ? `/${subject.path.split("/").map(encodeURIComponent).join("/")}`
      : "";
    return `http://127.0.0.1:${port}${candidatePath}?_k=${reloadKey()}`;
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
  const returnToReview = (candidate?: string) => {
    const run = executionId();
    if (!run) return;
    const session = ownerSessionId();
    const version = candidate ?? candidateId();
    closeArtifactCanvas();
    openCandidateReview(run, session, version);
  };
  const returnToConversation = () => {
    const sessionId = ownerSessionId();
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
        <header class="artifact-canvas-header" data-titlebar>
          <div class="artifact-canvas-title-group">
            <span class="artifact-canvas-badge">
              {draft() ? `Draft preview${candidateVersion() ? ` · Version ${candidateVersion()}` : ""}` : displayType() === "pdf" ? "PDF" : displayType() === "image" ? "Image" : displayType() === "table" ? "Data" : displayType() === "server" ? "Live preview" : displayType() === "office" ? "Document" : "Preview"}
            </span>
            <strong class="artifact-canvas-title">{title()}</strong>
            <Show when={path() && technicalDetails()}>
              <span class="artifact-canvas-path">{path()}</span>
            </Show>
            <Show when={canvasSubject()?.resultId}>{(resultId) =>
              <button type="button" class="artifact-canvas-result" title={resultId()} onClick={returnToConversation}>Back to the answer</button>
            }</Show>
          </div>

          <div class="artifact-canvas-controls">
            <Show when={displayType() === "pdf" && path()}>
              <div class="artifact-canvas-segmented">
                <button type="button" class="artifact-canvas-seg-btn" classList={{ active: !pdfText() }} onClick={() => setPdfText(false)} title="View the pages">Pages</button>
                <button type="button" class="artifact-canvas-seg-btn" classList={{ active: pdfText() }} onClick={() => setPdfText(true)} title="Read, comment on and edit the text">Text</button>
              </div>
            </Show>

            {/* View Mode Segmented Controls (for HTML and code artifacts) */}
            <Show when={displayType() === "html" || displayType() === "code" || displayType() === "table"}>
              <div class="artifact-canvas-segmented">
                <button
                  type="button"
                  class="artifact-canvas-seg-btn"
                  classList={{ active: viewMode() === "preview" }}
                  onClick={() => setViewMode("preview")}
                  title={displayType() === "table" ? "View data table" : "View interactive preview"}
                >
                  Preview
                </button>
                <button
                  type="button"
                  class="artifact-canvas-seg-btn"
                  classList={{ active: viewMode() === "source" }}
                  onClick={() => setViewMode("source")}
                  title="View source text"
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
                  title="Desktop"
                  aria-label="Desktop"
                  aria-pressed={device() === "desktop"}
                >
                  <Icon name="monitor" size={16} />
                </button>
                <button
                  type="button"
                  class="artifact-canvas-device-btn"
                  classList={{ active: device() === "tablet" }}
                  onClick={() => setCanvasDevice("tablet")}
                  title="Tablet"
                  aria-label="Tablet"
                  aria-pressed={device() === "tablet"}
                >
                  <Icon name="tablet" size={16} />
                </button>
                <button
                  type="button"
                  class="artifact-canvas-device-btn"
                  classList={{ active: device() === "mobile" }}
                  onClick={() => setCanvasDevice("mobile")}
                  title="Phone"
                  aria-label="Phone"
                  aria-pressed={device() === "mobile"}
                >
                  <Icon name="phone" size={16} />
                </button>
              </div>
            </Show>

            {/* Action Buttons */}
            <Show when={displayType() === "html" || displayType() === "server"}>
              <button type="button" class="artifact-canvas-btn" onClick={reload} title="Reload preview" aria-label="Reload preview"><Icon name="sync" size={14} /></button>
            </Show>
            <Show when={displayType() !== "office" && displayType() !== "pdf"}>
              <button type="button" class="artifact-canvas-btn" onClick={toggleCanvasMode} title={mode() === "split" ? "Expand to full width" : "Show beside conversation"} aria-label={mode() === "split" ? "Expand to full width" : "Show beside conversation"}><Icon name={mode() === "split" ? "layers" : "restore"} size={14} /></button>
            </Show>
            <Show when={displayType() === "html" || displayType() === "server"}>
              <button type="button" class="artifact-canvas-btn" onClick={popout} title="Open in new window" aria-label="Open in new window"><Icon name="preview" size={14} /></button>
            </Show>
            <Show when={candidateId() && executionId()}>
              <button type="button" class="artifact-canvas-btn" onClick={() => returnToReview()} title="Back to review">
                <Icon name="diff" size={14} /> Review changes
              </button>
            </Show>
            <Show when={displayType() === "office" || displayType() === "pdf"}>
              <button type="button" class="artifact-canvas-btn" aria-expanded={showFeedback()} onClick={() => setShowFeedback((show) => !show)}>{showFeedback() ? "Hide comments" : "Comment"}</button>
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
              <Show when={preparationRequired()}>
                <button type="button" class="artifact-canvas-btn" onClick={() => void preparePreview()}>
                  Prepare dependencies
                </button>
              </Show>
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
              <Show when={previewWarning()}>{(warning) =>
                <div class="artifact-canvas-preview-warning" role="status">{warning()}</div>
              }</Show>
              <Show when={displayType() === "table" && tablePreview()}>{(table) =>
                <div class="artifact-canvas-table-preview">
                  <p>{table().totalRows} {table().totalRows === 1 ? "row" : "rows"} · {table().headers.length} {table().headers.length === 1 ? "column" : "columns"}{table().truncated ? ` · showing first ${table().rows.length}` : ""}</p>
                  <div class="artifact-canvas-table-scroll">
                    <table>
                      <thead><tr><For each={table().headers}>{(header, index) => <th scope="col">{header || `Column ${index() + 1}`}</th>}</For></tr></thead>
                      <tbody><For each={table().rows}>{(row) => <tr><For each={row}>{(cell) => <td>{cell}</td>}</For></tr>}</For></tbody>
                    </table>
                  </div>
                </div>
              }</Show>
              <Show when={(displayType() === "office" || (displayType() === "pdf" && pdfText())) && canvasSubject()}>{(subject) =>
                <OfficeWorkspacePane
                  hideHeader
                  source={{ path: path(), sessionId: ownerSessionId(), candidateId: candidateId(), executionId: executionId() }}
                  fileName={title()}
                  focus={subject().anchor}
                  canEdit={true}
                  canStart={!!ownerSessionId() && !!candidateId()}
                  collaborators={coworkingPresence(ownerSessionId() ?? activeId())}
                  onReview={executionId() ? returnToReview : undefined}
                />
              }</Show>
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
              <Show when={displayType() === "pdf" && mediaUrl() && !pdfText()}>
                <iframe
                  class="artifact-canvas-pdf-frame"
                  src={pdfAnchorPage(canvasSubject()?.anchor) ? `${mediaUrl()}#page=${pdfAnchorPage(canvasSubject()?.anchor)}` : mediaUrl()!}
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
                      sandbox={previewSandbox(activeServerPort() ? "live_server" : "static")}
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

        <Show when={showFeedback()}><section class="artifact-canvas-feedback" aria-label="Review this draft">
          <div class="artifact-canvas-feedback-intro">
            <strong>Work on this together</strong>
            <span>Tell the Agent what to change in this draft.</span>
          </div>
          <div class="artifact-canvas-feedback-compose">
            <Show when={draft() && viewMode() === "source"}>
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
        </section></Show>

        {/* Security / Server Status Footer */}
        <Show when={displayType() !== "office" && displayType() !== "pdf"}><footer class="artifact-canvas-footer">
          <span class="artifact-canvas-security">
            <Icon name="shield" size={12} />
            <Show
              when={activeServerPort()}
              fallback={
                <>
                  Safe preview, offline
                </>
              }
            >
              Live preview: {`http://127.0.0.1:${activeServerPort()}`}
            </Show>
          </span>
        </footer></Show>
      </div>
    </Show>
  );
}
