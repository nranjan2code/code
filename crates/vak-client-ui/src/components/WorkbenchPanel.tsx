import { createEffect, createMemo, createSignal, For, Index, Show, onCleanup } from "solid-js";
import {
  workbenchExecutions,
  workbenchLoadError,
  activeExecutionId,
  setActiveExecutionId,
  setWorkbenchExecutions,
  type WorkbenchExecution,
  activeId,
  requestedArtifact,
  setRequestedArtifact,
  openArtifactCanvas,
  openArtifactFile,
  workbenchTab,
  setWorkbenchTab,
  candidateReviewRequest,
  setCandidateReviewRequest,
  technicalDetails,
} from "../store";
import * as api from "../api";
import { watchCoworking } from "../streamHub";
import Icon from "./Icon";
import Sheet from "./Sheet";
import OfficeChangeList from "./OfficeChangeList";
import OfficeView from "./OfficeView";
import { isDocumentPath, isOfficePath } from "../officeFiles";
import { acceptanceSummary, pendingVersions, undoablePromotion } from "../candidateVersions";
import { keep as keepChoice, kept as keptChoices, leaveOut } from "../officeChoices";
import { artifactPreviewHtml } from "../artifactPreview";
import { sandboxedSrcdoc } from "../safeUrl";
import { fileSubject, runOrigin } from "../canvasSubject";
import { activate } from "../App";
import Skeleton from "./Skeleton";

function formatBytes(bytes?: number): string {
  if (!bytes || bytes <= 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function foldCarriageReturns(text: string): string {
  if (!text.includes("\r")) return text;
  const lines = text.split("\n");
  const result: string[] = [];
  for (const line of lines) {
    if (line.includes("\r")) {
      const parts = line.split("\r").filter((p) => p.length > 0);
      result.push(parts[parts.length - 1] ?? "");
    } else {
      result.push(line);
    }
  }
  return result.join("\n");
}

function renderAnsiToHtml(rawText: string): string {
  if (!rawText) return "";
  const text = foldCarriageReturns(rawText);
  const ansiRegex = /\x1b\[([0-9;]*)m/g;
  let html = "";
  let currentIndex = 0;
  let openSpans = 0;

  const colorMap: Record<string, string> = {
    "30": "#64748b",
    "31": "#f87171",
    "32": "#4ade80",
    "33": "#facc15",
    "34": "#60a5fa",
    "35": "#c084fc",
    "36": "#22d3ee",
    "37": "#e2e8f0",
    "90": "#94a3b8",
    "91": "#fca5a5",
    "92": "#86efac",
    "93": "#fde047",
    "94": "#93c5fd",
    "95": "#d8b4fe",
    "96": "#67e8f9",
    "97": "#ffffff",
  };

  function escapeHtml(str: string): string {
    return str
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
  }

  let match: RegExpExecArray | null;
  while ((match = ansiRegex.exec(text)) !== null) {
    const rawChunk = text.slice(currentIndex, match.index);
    if (rawChunk) {
      html += escapeHtml(rawChunk);
    }
    currentIndex = ansiRegex.lastIndex;

    const codes = match[1] ? match[1].split(";") : ["0"];
    for (const code of codes) {
      if (code === "0" || code === "") {
        while (openSpans > 0) {
          html += "</span>";
          openSpans--;
        }
      } else if (code === "1") {
        html += '<span style="font-weight: 600;">';
        openSpans++;
      } else if (code === "2") {
        html += '<span style="opacity: 0.7;">';
        openSpans++;
      } else if (colorMap[code]) {
        html += `<span style="color: ${colorMap[code]};">`;
        openSpans++;
      }
    }
  }

  const remaining = text.slice(currentIndex);
  if (remaining) {
    html += escapeHtml(remaining);
  }
  while (openSpans > 0) {
    html += "</span>";
    openSpans--;
  }

  return html;
}

export default function WorkbenchPanel() {
  const tab = workbenchTab;
  const setTab = setWorkbenchTab;
  const [selectedArtifact, setSelectedArtifact] = createSignal<string | null>(null);
  const [browsingFiles, setBrowsingFiles] = createSignal(false);
  const [showRunDetails, setShowRunDetails] = createSignal(false);
  const [reviewFocus, setReviewFocus] = createSignal(false);
  const [artifactPreview, setArtifactPreview] = createSignal("");
  const [artifactContent, setArtifactContent] = createSignal<string | null>(null);
  const [artifactDataUrl, setArtifactDataUrl] = createSignal<string | null>(null);
  const [loadingArtifact, setLoadingArtifact] = createSignal(false);
  const [artifactError, setArtifactError] = createSignal<string | null>(null);
  const [copiedCmd, setCopiedCmd] = createSignal(false);
  const [copiedLog, setCopiedLog] = createSignal(false);
  const [stopping, setStopping] = createSignal(false);
  const [candidate, setCandidate] = createSignal<Awaited<ReturnType<typeof api.exportSandboxCandidate>> | null>(null);
  const [pendingCandidates, setPendingCandidates] = createSignal<api.SandboxCandidateRecord[]>([]);
  const [candidateBusy, setCandidateBusy] = createSignal(false);
  const [promotionMessage, setPromotionMessage] = createSignal<string | null>(null);
  // The session's durable sandbox records: what an execution's acceptance
  // can still undo is derived from these, never from what this page did.
  const [sandboxRecords, setSandboxRecords] = createSignal<api.SandboxRecord[]>([]);
  const [workspaceCheckBusy, setWorkspaceCheckBusy] = createSignal<string | null>(null);
  const [undoBusy, setUndoBusy] = createSignal(false);
  const [reviewOpen, setReviewOpen] = createSignal(false);
  const [reviewedPath, setReviewedPath] = createSignal<string | null>(null);
  const [officeReview, setOfficeReview] = createSignal<api.OfficeReview | null>(null);
  // Choices the person left out of the Office draft under review; Accept
  // waits until they are made into a version or taken back.
  const [officeExcluded, setOfficeExcluded] = createSignal<ReadonlySet<string>>(new Set());
  const [officeNarrowBusy, setOfficeNarrowBusy] = createSignal(false);
  const [officeNarrowError, setOfficeNarrowError] = createSignal<string | null>(null);
  const [reviewedFiles, setReviewedFiles] = createSignal<string[]>([]);
  const [inspectedFiles, setInspectedFiles] = createSignal<string[]>([]);
  const [beforeContent, setBeforeContent] = createSignal<string | null>(null);
  const [afterContent, setAfterContent] = createSignal<string | null>(null);
  // Review shows an HTML draft as the page it is, offline and in a sandbox,
  // before the source and the comparison. A signal, not a resource: reading
  // a pending resource here would suspend the dock and remount the sheet.
  const [draftPage, setDraftPage] = createSignal<string | null>(null);
  createEffect(() => {
    const version = candidate();
    const path = reviewedPath();
    const content = afterContent();
    setDraftPage(null);
    if (!version || !path || content === null || !/\.html?$/i.test(path)) return;
    let current = true;
    onCleanup(() => { current = false; });
    void artifactPreviewHtml(path, content, {
      readFile: (file) => api.readSandboxCandidateFile(version.session_id, version.candidate.candidate_id, file),
      readFileRaw: (file) => api.readSandboxCandidateFileRaw(version.session_id, version.candidate.candidate_id, file),
    })
      .catch(() => sandboxedSrcdoc(content))
      .then((page) => { if (current) setDraftPage(page); });
  });
  const [reviewFileError, setReviewFileError] = createSignal<string | null>(null);
  const [reviewComment, setReviewComment] = createSignal("");
  // The Office anchor the next comment points at (docs/design/72, F9).
  const [commentAnchor, setCommentAnchor] = createSignal<string | null>(null);
  const [reviewCommentBusy, setReviewCommentBusy] = createSignal(false);
  const [reviewCommentMessage, setReviewCommentMessage] = createSignal<string | null>(null);
  const [previewPreparations, setPreviewPreparations] = createSignal<api.SandboxPreviewPreparationRecord[]>([]);

  let recordsRequest = 0;
  const loadSandboxRecords = async (sessionId: string) => {
    const request = ++recordsRequest;
    const { records } = await api.listSessionSandboxRecords(sessionId);
    if (request === recordsRequest && activeId() === sessionId) setSandboxRecords(records);
    return records;
  };
  // Shows a record the server just returned at once, then reloads so the
  // list stays exactly what the server holds.
  const recordAppended = (sessionId: string, record: api.SandboxRecord) => {
    recordsRequest += 1;
    if (activeId() === sessionId) setSandboxRecords((current) => [...current, record]);
    void loadSandboxRecords(sessionId).catch(() => { /* The appended record stands until the next load. */ });
  };
  createEffect(() => {
    const sessionId = activeId();
    recordsRequest += 1;
    setSandboxRecords([]);
    if (sessionId) void loadSandboxRecords(sessionId).catch(() => { /* Undo remains hidden when durable state is unavailable. */ });
  });
  const acceptanceFor = (executionId: string) => undoablePromotion(sandboxRecords(), executionId);
  const acceptanceMessage = (executionId: string) => {
    const applied = acceptanceFor(executionId);
    return applied ? acceptanceSummary(applied) : null;
  };
  const workspaceCheckReceipts = (candidateId: string) => sandboxRecords().flatMap((record) => record.kind === "WorkspaceCheck" && record.record.candidate_id === candidateId ? [record.record] : []);

  const [candidateComments, setCandidateComments] = createSignal<api.SandboxCandidateComment[]>([]);
  const [controlError, setControlError] = createSignal<string | null>(null);
  const [pulse, setPulse] = createSignal(0);

  // A quiet interval is still meaningful feedback while the provider or a
  // tool is between events. It also makes the status accessible to screen
  // readers without duplicating the transcript. Only run it while something
  // is actually running so idle panels don't force re-renders forever.
  createEffect(() => {
    if (!isAnyRunning()) return;
    const pulseTimer = window.setInterval(() => setPulse((value) => value + 1), 1000);
    onCleanup(() => window.clearInterval(pulseTimer));
  });
  onCleanup(() => {
    artifactRequest += 1;
    const url = artifactDataUrl();
    if (url?.startsWith("blob:")) URL.revokeObjectURL(url);
  });

  let terminalRef: HTMLDivElement | undefined;
  let artifactRequest = 0;

  createEffect(() => {
    void activeId();
    artifactRequest += 1;
    setBrowsingFiles(false);
    setSelectedArtifact(null);
    setArtifactContent(null);
    setArtifactError(null);
    setCandidate(null);
    setPendingCandidates([]);
    setPromotionMessage(null);
    setReviewOpen(false);
    setReviewedPath(null);
    setReviewedFiles([]);
    setInspectedFiles([]);
    setReviewComment("");
    setReviewCommentMessage(null);
    setCandidateComments([]);
    setPreviewPreparations([]);
  });

  const executions = () => workbenchExecutions();
  const currentExec = () => {
    const active = activeExecutionId();
    if (active) {
      const found = executions().find((e) => e.id === active);
      if (found) return found;
    }
    return executions()[executions().length - 1] ?? null;
  };

  createEffect((previous: string | null) => {
    const exec = currentExec();
    const key = exec ? `${activeId()}:${exec.id}` : null;
    if (key && key !== previous && exec?.scratchDir) {
      let disposed = false;
      const sessionId = activeId();
      if (!sessionId) return key;
      void loadSandboxRecords(sessionId).then((records) => {
        if (disposed) return;
        if (reviewOpen() && candidate()?.execution_id !== exec.id) return;
        const pending = pendingVersions(records, exec.id);
        setPendingCandidates(pending);
        if (!reviewOpen() && !pending.some((record) => record.candidate.candidate_id === candidate()?.candidate.candidate_id)) {
          if (pending.length > 0) selectCandidate(pending[pending.length - 1]);
          else setCandidate(null);
        }
      }).catch(() => { /* Review remains available by preparing it again. */ });
      onCleanup(() => { disposed = true; });
    }
    return key;
  }, null);

  createEffect(() => {
    const cur = currentExec();
    if (cur && (cur.stdout || cur.stderr) && terminalRef) {
      terminalRef.scrollTop = terminalRef.scrollHeight;
    }
  });

  const allArtifacts = () => {
    const list: Array<{
      path: string;
      mimeType: string;
      sizeBytes: number;
      execCommand: string;
      timestamp: string;
      executionId: string;
      sessionId?: string;
      revision?: number;
    }> = [];
    for (const e of executions()) {
      for (const a of e.artifacts) {
        list.push({
          ...a,
          execCommand: e.command,
          timestamp: e.timestamp,
          executionId: e.id,
          sessionId: e.ownerSessionId ?? activeId() ?? undefined,
        });
      }
    }
    return [...new Map(list.map((artifact) => [artifact.path, artifact])).values()];
  };
  const artifactSource = (path: string) => allArtifacts().find((artifact) => artifact.path === path);

  const isAnyRunning = () => executions().some((e) => e.status === "running");
  const runningExec = () => executions().find((e) => e.status === "running");

  const clearExecutions = () => {
    setWorkbenchExecutions([]);
    setActiveExecutionId(null);
    setSelectedArtifact(null);
  };

  const copyCommand = async (cmd: string) => {
    try {
      await navigator.clipboard.writeText(cmd);
      setCopiedCmd(true);
      setTimeout(() => setCopiedCmd(false), 2000);
    } catch (error) {
      setControlError(error instanceof Error ? error.message : String(error));
    }
  };

  const copyLog = async (exec: WorkbenchExecution) => {
    setControlError(null);
    try {
      const fullLog = `${exec.stdout ? `[stdout]\n${exec.stdout}\n` : ""}${
        exec.stderr ? `[stderr]\n${exec.stderr}\n` : ""
      }`;
      await navigator.clipboard.writeText(fullLog);
      setCopiedLog(true);
      setTimeout(() => setCopiedLog(false), 2000);
    } catch (error) {
      setControlError(error instanceof Error ? error.message : String(error));
    }
  };

  const stopExecution = async () => {
    const sid = activeId();
    if (!sid) return;
    setStopping(true);
    setControlError(null);
    try {
      await api.cancelRun(sid);
    } catch (error) {
      setControlError(error instanceof Error ? error.message : String(error));
    } finally {
      setTimeout(() => setStopping(false), 1000);
    }
  };

  const inspectArtifact = async (path: string) => {
    const request = ++artifactRequest;
    // Artifact links are an intent to inspect, so move to the artifact
    // viewer immediately instead of leaving the operator on live logs.
    setTab("artifacts");
    setBrowsingFiles(false);
    setSelectedArtifact(path);
    setLoadingArtifact(true);
    setArtifactError(null);
    setArtifactContent(null);
    const previousUrl = artifactDataUrl();
    if (previousUrl?.startsWith("blob:")) URL.revokeObjectURL(previousUrl);
    setArtifactDataUrl(null);
    const source = artifactSource(path);
    const readFile = (file: string) => source?.sessionId
      ? api.readExecutionArtifact(source.sessionId, source.executionId, file)
      : api.readFile(file);
    const readRaw = (file: string) => source?.sessionId
      ? api.readExecutionArtifactRaw(source.sessionId, source.executionId, file)
      : api.readFileRaw(file);
    try {
      if (isOfficePath(path)) {
        // The Office view reads the file through the server's worker; the
        // bytes are fetched only so the file can be downloaded.
        const rawUrl = await readRaw(path);
        if (request !== artifactRequest) {
          URL.revokeObjectURL(rawUrl);
          return;
        }
        setArtifactDataUrl(rawUrl);
        return;
      }
      const res = await readFile(path);
      if (request !== artifactRequest) return;
      if (res.data_url) {
        setArtifactDataUrl(res.data_url);
      } else if (res.content !== undefined) {
        const preview = /\.html?$/i.test(path) ? await artifactPreviewHtml(path, res.content, { readFile, readFileRaw: readRaw }) : "";
        if (request !== artifactRequest) return;
        setArtifactContent(res.content);
        setArtifactPreview(preview);
      } else {
        // Large/opaque formats use the authenticated raw endpoint instead of
        // forcing every renderer through a base64 JSON response.
        const rawUrl = await readRaw(path);
        if (request !== artifactRequest) {
          URL.revokeObjectURL(rawUrl);
          return;
        }
        setArtifactDataUrl(rawUrl);
      }
    } catch (err) {
      if (request !== artifactRequest) return;
      setArtifactError(err instanceof Error ? err.message : String(err));
    } finally {
      if (request === artifactRequest) setLoadingArtifact(false);
    }
  };

  createEffect(() => {
    const path = requestedArtifact();
    if (path) { void inspectArtifact(path); setRequestedArtifact(null); }
  });

  const artifactVersion = createMemo(() => allArtifacts().find((artifact) => artifact.path === selectedArtifact())?.revision ?? 0);
  let shownVersion = 0;
  createEffect(() => {
    const version = artifactVersion();
    const path = selectedArtifact();
    if (path && version > shownVersion) void inspectArtifact(path);
    shownVersion = version;
  });

  // A produced file is the outcome of the turn, not an implementation detail.
  // Open the newest outcome automatically when the panel has no selection;
  // users can still switch to activity when they want the mechanics.
  createEffect(() => {
    const artifacts = allArtifacts();
    if (artifacts.length > 0 && selectedArtifact() === null && !browsingFiles()) {
      void inspectArtifact(artifacts[artifacts.length - 1].path);
    }
  });

  const isHtmlArtifact = (path: string) =>
    /\.html?$/i.test(path);

  const isImageArtifact = (path: string, mime?: string) =>
    mime?.startsWith("image/") ||
    /\.(png|jpg|jpeg|gif|svg|webp)$/i.test(path);

  const isPdfArtifact = (path: string, mime?: string) =>
    mime === "application/pdf" || path.toLowerCase().endsWith(".pdf");

  const isAudioArtifact = (path: string, mime?: string) =>
    mime?.startsWith("audio/") || /\.(mp3|wav|ogg|m4a|aac)$/i.test(path);

  const isVideoArtifact = (path: string, mime?: string) =>
    mime?.startsWith("video/") || /\.(mp4|webm|mov|m4v)$/i.test(path);

  function selectCandidate(prepared: api.SandboxCandidateRecord) {
    setCandidate(prepared);
    setReviewedFiles(prepared.candidate.files.map((file) => file.path));
    setInspectedFiles([]);
    setReviewedPath(prepared.candidate.files[0]?.path ?? null);
    setReviewFileError(null);
    setBeforeContent(null);
    setAfterContent(null);
    setReviewCommentMessage(null);
    setCandidateComments([]);
    void api.listSandboxCandidateComments(prepared.session_id, prepared.candidate.candidate_id)
      .then(({ comments }) => {
        if (candidate()?.candidate.candidate_id === prepared.candidate.candidate_id) setCandidateComments(comments);
      })
      .catch(() => { /* Review remains usable if comment history is unavailable. */ });
  }

  createEffect(() => {
    const prepared = candidate();
    const sessionId = activeId();
    if (!reviewOpen() || !prepared || !sessionId || prepared.session_id !== sessionId) return;
    let disposed = false;
    const refresh = () => {
      void api.listSandboxCandidateComments(sessionId, prepared.candidate.candidate_id)
        .then(({ comments }) => {
          if (!disposed && candidate()?.candidate.candidate_id === prepared.candidate.candidate_id) setCandidateComments(comments);
        })
        .catch(() => { /* Keep the last known comments during a connection failure. */ });
      void loadSandboxRecords(sessionId)
        .then((records) => {
          if (disposed || candidate()?.candidate.candidate_id !== prepared.candidate.candidate_id) return;
          setPreviewPreparations(records.filter((record): record is { kind: "PreviewPreparation"; record: api.SandboxPreviewPreparationRecord } => record.kind === "PreviewPreparation").map((record) => record.record));
          const versions = pendingVersions(records, prepared.execution_id);
          setPendingCandidates(versions);
          // A version the person made by keeping some changes is opened by
          // that action itself; only an Agent revision arrives here.
          const next = versions.findLast((record) => record.parent_candidate_id === prepared.candidate.candidate_id && !record.narrowed);
          if (next) {
            selectCandidate(next);
            setReviewCommentMessage("The Agent prepared a new draft version. Review its files before accepting.");
            return;
          }
          const revision = records.filter((record): record is { kind: "CandidateRevision"; record: api.SandboxCandidateRevisionRecord } => record.kind === "CandidateRevision" && record.record.parent_candidate_id === prepared.candidate.candidate_id).at(-1)?.record;
          if (revision?.status === "Failed") setReviewCommentMessage(`Could not prepare a new version: ${revision.detail ?? "Agent revision failed"}`);
        })
        .catch(() => { /* Keep the current reviewed version while offline. */ });
    };
    refresh();
    const stop = watchCoworking(sessionId, refresh);
    onCleanup(() => { disposed = true; stop(); });
  });

  const reviewCandidate = async (requestedCandidateId?: string) => {
    const exec = currentExec();
    const sessionId = activeId();
    if (!sessionId || !exec || exec.artifacts.length === 0) return;
    let saved: api.SandboxCandidateRecord[];
    try {
      const records = await loadSandboxRecords(sessionId);
      if (activeId() !== sessionId) return;
      saved = pendingVersions(records, exec.id);
      setPendingCandidates(saved);
    } catch (error) {
      setPromotionMessage(error instanceof Error ? error.message : String(error));
      return;
    }
    if (saved.length > 0) {
      const requested = requestedCandidateId ? saved.find((record) => record.candidate.candidate_id === requestedCandidateId) : undefined;
      if (requested) selectCandidate(requested);
      else if (!saved.some((record) => record.candidate.candidate_id === candidate()?.candidate.candidate_id)) selectCandidate(saved[saved.length - 1]);
      setReviewOpen(true);
      return;
    }
    setCandidateBusy(true);
    setPromotionMessage(null);
    try {
      const prepared = await api.exportSandboxCandidate(sessionId, exec.id, exec.scratchDir, ".");
      if (activeId() !== sessionId) return;
      setPendingCandidates((current) => [...current, prepared]);
      selectCandidate(prepared);
      setReviewOpen(true);
    } catch (error) {
      setPromotionMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setCandidateBusy(false);
    }
  };

  createEffect(() => {
    const requested = candidateReviewRequest();
    if (!requested) return;
    if (requested.sessionId && requested.sessionId !== activeId()) {
      void activate(requested.sessionId);
      return;
    }
    if (currentExec()?.id !== requested.executionId) return;
    setCandidateReviewRequest(null);
    void reviewCandidate(requested.candidateId);
  });

  // Only an execution that ran inside `.vak/scratch/` holds a draft to review
  // and accept. A command that worked in the workspace already wrote its files
  // where they belong, and the server refuses to export it as a candidate.
  const ranInScratch = (exec: { scratchDir: string }) => /(^|[\\/])\.vak[\\/]scratch([\\/]|$)/.test(exec.scratchDir);
  const candidatePath = (root: string, path: string) => `${root.replace(/\/$/, "")}/${path}`;
  const fileState = (file: { candidate_hash: string; base_hash?: string; operation?: "Upsert" | "Delete" }) =>
    file.operation === "Delete" ? "Deleted" : !file.base_hash ? "New" : file.base_hash === file.candidate_hash ? "Unchanged" : "Changed";
  const candidateSummary = createMemo(() => {
    const files = candidate()?.candidate.files ?? [];
    return {
      newFiles: files.filter((file) => fileState(file) === "New").length,
      changedFiles: files.filter((file) => fileState(file) === "Changed").length,
      deletedFiles: files.filter((file) => fileState(file) === "Deleted").length,
      unchangedFiles: files.filter((file) => fileState(file) === "Unchanged").length,
    };
  });
  const destinationLabel = () => {
    const root = candidate()?.candidate.destination_root.replace(/\/$/, "") ?? "";
    return root.split("/").filter(Boolean).pop() || root || "workspace";
  };
  const candidateVersion = () => {
    const prepared = candidate();
    if (!prepared) return 0;
    return pendingCandidates()
      .filter((record) => record.execution_id === prepared.execution_id)
      .findIndex((record) => record.candidate.candidate_id === prepared.candidate.candidate_id) + 1;
  };

  createEffect(() => {
    const prepared = candidate();
    const path = reviewedPath();
    if (!reviewOpen() || !prepared || !path) return;
    let disposed = false;
    setBeforeContent(null);
    setAfterContent(null);
    setReviewFileError(null);
    setOfficeReview(null);
    setCommentAnchor(null);
    setOfficeExcluded(new Set<string>());
    setOfficeNarrowError(null);
    const reviewedFile = prepared.candidate.files.find((file) => file.path === path);
    if (isDocumentPath(path) && reviewedFile?.operation !== "Delete") {
      void api.readSandboxCandidateOfficeReview(prepared.session_id, prepared.candidate.candidate_id, path)
        .then((review) => {
          if (disposed) return;
          setOfficeReview(review);
          setInspectedFiles((paths) => paths.includes(path) ? paths : [...paths, path]);
        })
        .catch(() => {
          if (!disposed) setReviewFileError("Could not compare this draft. Review is unavailable until it can be read.");
        });
      onCleanup(() => { disposed = true; });
      return;
    }
    const afterRequest = reviewedFile?.operation === "Delete"
      ? Promise.resolve({ content: null })
      : api.readSandboxCandidateFile(prepared.session_id, prepared.candidate.candidate_id, path);
    void Promise.allSettled([
      api.readFile(candidatePath(prepared.candidate.destination_root, path)),
      afterRequest,
    ]).then(([before, after]) => {
      if (disposed) return;
      setBeforeContent(before.status === "fulfilled" ? before.value.content ?? null : null);
      setAfterContent(after.status === "fulfilled" ? after.value.content ?? null : null);
      if (after.status === "rejected") setReviewFileError("Could not load this draft file. Review is unavailable until it can be read.");
      else setInspectedFiles((paths) => paths.includes(path) ? paths : [...paths, path]);
    });
    onCleanup(() => { disposed = true; });
  });

  const toggleOfficeChoice = (id: string) => {
    const choices = officeReview()?.choices ?? [];
    setOfficeExcluded((excluded) => excluded.has(id) ? keepChoice(choices, excluded, id) : leaveOut(choices, excluded, id));
  };

  const makeOfficeVersion = async () => {
    const prepared = candidate();
    const review = officeReview();
    const path = reviewedPath();
    if (!prepared || !review?.choices || !path || officeNarrowBusy()) return;
    const keep = keptChoices(review.choices, officeExcluded());
    if (keep.length === 0 || keep.length === review.choices.length) return;
    setOfficeNarrowBusy(true);
    setOfficeNarrowError(null);
    try {
      const version = await api.narrowSandboxCandidateOffice(prepared.session_id, prepared.candidate.candidate_id, path, keep);
      setPendingCandidates((current) => [...current, version]);
      selectCandidate(version);
      setReviewedPath(path);
    } catch (error) {
      setOfficeNarrowError(`Could not make that version: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setOfficeNarrowBusy(false);
    }
  };

  const openCandidateVersion = (candidateId: string) => {
    const path = reviewedPath();
    const version = pendingCandidates().find((record) => record.candidate.candidate_id === candidateId);
    if (!version) return;
    selectCandidate(version);
    if (path) setReviewedPath(path);
  };

  const toggleReviewedFile = (path: string) => {
    setReviewedFiles((paths) => paths.includes(path) ? paths.filter((entry) => entry !== path) : [...paths, path]);
  };

  const sendReviewComment = async () => {
    const id = activeId();
    const prepared = candidate();
    const comment = reviewComment().trim();
    if (!id || !prepared || !comment) return;
    setReviewCommentBusy(true);
    setReviewCommentMessage(null);
    try {
      await api.commentOnSandboxCandidate(id, prepared.candidate.candidate_id, comment, { path: reviewedPath() ?? undefined, anchor: commentAnchor() ?? undefined });
      setReviewComment("");
      setCommentAnchor(null);
      setReviewCommentMessage("Comment saved on this draft. Agent revision will be available after isolated draft editing is ready.");
      void api.listSandboxCandidateComments(id, prepared.candidate.candidate_id)
        .then(({ comments }) => setCandidateComments(comments))
        .catch(() => { /* The accepted comment remains durable. */ });
    } catch (error) {
      setReviewCommentMessage(`Could not send feedback: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setReviewCommentBusy(false);
    }
  };

  const requestRevisionFromComment = async (commentId: string) => {
    const id = activeId();
    const prepared = candidate();
    if (!id || !prepared || reviewCommentBusy()) return;
    setReviewCommentBusy(true);
    setReviewCommentMessage(null);
    try {
      await api.requestRevisionFromCandidateComment(id, prepared.candidate.candidate_id, commentId);
      setReviewCommentMessage("The Agent is preparing a new version in its isolated draft. The saved version stays available for review.");
    } catch (error) {
      setReviewCommentMessage(`Could not request a revision: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setReviewCommentBusy(false);
    }
  };

  const promoteCandidate = async () => {
    const value = candidate();
    if (!value || reviewedFiles().length === 0 || reviewedFiles().some((path) => !inspectedFiles().includes(path)) || reviewFileError() || officeExcluded().size > 0) return;
    setCandidateBusy(true);
    try {
      const receipt = await api.promoteSandboxCandidate(value.session_id, value.candidate.candidate_id, reviewedFiles());
      setPromotionMessage(null);
      recordAppended(value.session_id, { kind: "Promotion", record: receipt });
      setReviewOpen(false);
      setPendingCandidates((current) => current.filter((record) => record.execution_id !== value.execution_id));
      setCandidate(null);
    } catch (error) {
      setPromotionMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setCandidateBusy(false);
    }
  };

  const undoPromotion = async (applied: api.SandboxPromotionRecord) => {
    const id = activeId();
    if (!id || undoBusy()) return;
    setUndoBusy(true);
    try {
      const undone = await api.undoSandboxPromotion(id, applied.candidate_id);
      setPromotionMessage(`Restored ${undone.receipt.restored.length} file(s) to their pre-acceptance state.`);
      recordAppended(id, { kind: "PromotionUndo", record: undone });
    } catch (error) {
      setPromotionMessage(`Could not undo: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setUndoBusy(false);
    }
  };

  const runWorkspaceCheck = async (applied: api.SandboxPromotionRecord, check: api.WorkspaceCheckPlan) => {
    const id = activeId();
    if (!id || workspaceCheckBusy()) return;
    setWorkspaceCheckBusy(check.id);
    try {
      const receipt = await api.runSandboxWorkspaceCheck(id, applied.candidate_id, check.id);
      recordAppended(id, { kind: "WorkspaceCheck", record: receipt });
      setPromotionMessage(`${check.label} ${receipt.status}.`);
    } catch (error) {
      setPromotionMessage(`Could not run ${check.label}: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setWorkspaceCheckBusy(null);
    }
  };

  return (
    <div class="workbench-panel">
      <Show when={reviewOpen() && candidate()}>
        {(prepared) => <Sheet
          size="wide"
          class={reviewFocus() ? `candidate-review candidate-review-focused${isDocumentPath(reviewedPath() ?? "") ? " candidate-review-document-focused" : ""}` : "candidate-review"}
          title="Review changes"
          subtitle={`Nothing changes in ${destinationLabel()} until you accept.`}
          onClose={() => setReviewOpen(false)}
          busy={candidateBusy()}
          footer={<>
              <span>{officeExcluded().size > 0 ? "Make a version with the changes you kept, or keep every change, before accepting." : `Applying ${reviewedFiles().length} selected ${reviewedFiles().length === 1 ? "change" : "changes"} to ${destinationLabel()} · ${reviewedFiles().filter((path) => inspectedFiles().includes(path)).length} viewed`}</span>
              <button type="button" class="btn" onClick={() => setReviewOpen(false)}>Keep as draft</button>
              <button type="button" class="btn primary" disabled={candidateBusy() || reviewedFiles().length === 0 || reviewedFiles().some((path) => !inspectedFiles().includes(path)) || !!reviewFileError() || officeExcluded().size > 0} onClick={() => void promoteCandidate()}>{candidateBusy() ? "Accepting…" : `Accept ${reviewedFiles().length} selected ${reviewedFiles().length === 1 ? "file" : "files"}`}</button>
          </>}
        >
            <div>
              <section class="candidate-review-section" aria-label="Preview">
                <div class="candidate-review-section-head">
                  <h3>{reviewedPath()?.split("/").pop() ?? "Choose a file"}</h3>
                  <button type="button" class="btn sm" aria-pressed={reviewFocus()} onClick={() => setReviewFocus((focused) => !focused)}>{reviewFocus() ? "Show full review" : "Focus on this file"}</button>
                  <Show when={reviewedPath()}>{(path) =>
                    <Show when={prepared().candidate.files.find((file) => file.path === path())?.operation !== "Delete"}><button type="button" class="btn" disabled={!!reviewFileError()} onClick={() => {
                      const version = prepared();
                      setReviewOpen(false);
                      openArtifactCanvas(fileSubject(path(), {
                        sessionId: version.session_id,
                        resultId: version.result_id,
                        executionId: version.execution_id,
                        candidateId: version.candidate.candidate_id,
                      }, path().split("/").pop() || path()));
                    }}>Open saved version in Canvas</button></Show>
                  }</Show>
                </div>
                <Show when={reviewFileError()}>{(message) => <p role="alert" class="inline-error">{message()}</p>}</Show>
                <Show when={draftPage() && !reviewFileError()}>
                  <iframe class="candidate-review-page" title={`Preview of ${reviewedPath()?.split("/").pop() ?? "the draft"}`} sandbox="allow-scripts" srcdoc={draftPage() ?? ""} />
                </Show>
                <Show when={!draftPage() && reviewedPath() && !reviewFileError() && !isDocumentPath(reviewedPath() ?? "")}>
                  <pre class="candidate-review-draft">{prepared().candidate.files.find((file) => file.path === reviewedPath())?.operation === "Delete" ? "This file will be deleted." : afterContent() ?? "Loading or preview unavailable"}</pre>
                </Show>
                <Show when={reviewedPath() && !reviewFileError() && isDocumentPath(reviewedPath() ?? "")}>
                  <p class="office-change-empty">Open the saved version to see the whole document. The changes are listed below.</p>
                </Show>
              </section>
              <section class="candidate-review-section" aria-label="Changes">
                <h3>Changes</h3>
                <Show when={reviewFocus() && (reviewFileError() || (prepared().draft_checks ?? []).some((check) => check.status !== "passed"))}>
                  <p role="alert" class="inline-error">This draft needs attention before you can accept it. Show full review for details.</p>
                </Show>
                <Show when={pendingCandidates().filter((record) => record.execution_id === prepared().execution_id).length > 1}>
                  <label for="candidate-review-version">Draft version</label>
                  <select id="candidate-review-version" onChange={(event) => {
                    const selected = pendingCandidates().find((record) => record.candidate.candidate_id === event.currentTarget.value);
                    if (selected) selectCandidate(selected);
                  }}>
                    <For each={pendingCandidates().filter((record) => record.execution_id === prepared().execution_id)}>{(record, index) =>
                      <option value={record.candidate.candidate_id} selected={record.candidate.candidate_id === prepared().candidate.candidate_id}>Version {index() + 1}{record.narrowed ? ` · keeps ${record.narrowed.keep.length} ${record.narrowed.keep.length === 1 ? "change" : "changes"}` : ""} · {new Date(record.updated_at).toLocaleString()}</option>
                    }</For>
                  </select>
                </Show>
                <div class="candidate-review-counts">
                  <strong>{candidateSummary().newFiles} new</strong>
                  <strong>{candidateSummary().changedFiles} changed</strong>
                  <Show when={candidateSummary().deletedFiles > 0}><strong>{candidateSummary().deletedFiles} deleted</strong></Show>
                  <Show when={candidateSummary().unchangedFiles > 0}><span>{candidateSummary().unchangedFiles} unchanged</span></Show>
                </div>
                <div class="candidate-review-files" aria-label="Draft files">
                  <For each={prepared().candidate.files}>{(file) => <div class="candidate-review-file">
                    <input type="checkbox" aria-label={`Apply ${file.path}`} checked={reviewedFiles().includes(file.path)} onChange={() => toggleReviewedFile(file.path)} />
                    <button type="button" classList={{ active: reviewedPath() === file.path }} onClick={() => setReviewedPath(file.path)}>{file.path}</button>
                    <span>{fileState(file)}{inspectedFiles().includes(file.path) ? " · Viewed" : ""}</span>
                  </div>}</For>
                </div>
                <Show when={reviewedPath() && !reviewFileError() && !isDocumentPath(reviewedPath() ?? "")}>
                  <details class="candidate-review-compare">
                    <summary>Compare with the current file</summary>
                    <div class="candidate-review-columns">
                      <div><strong>Now</strong><pre>{beforeContent() ?? "New file or preview unavailable"}</pre></div>
                      <div><strong>After</strong><pre>{prepared().candidate.files.find((file) => file.path === reviewedPath())?.operation === "Delete" ? "This file will be deleted." : afterContent() ?? "Loading or preview unavailable"}</pre></div>
                    </div>
                  </details>
                </Show>
                <Show when={reviewedPath() && !reviewFileError() && isDocumentPath(reviewedPath() ?? "")}>
                  <Show when={officeReview()} fallback={<p class="office-change-empty">Comparing…</p>}>{(review) => <OfficeChangeList
                    review={review()}
                    excluded={officeExcluded()}
                    onToggle={toggleOfficeChoice}
                    onMakeVersion={() => void makeOfficeVersion()}
                    busy={officeNarrowBusy()}
                    error={officeNarrowError()}
                    onOpenVersion={openCandidateVersion}
                    onComment={(anchor) => {
                      setCommentAnchor(anchor);
                      document.getElementById("candidate-review-comment")?.focus();
                    }}
                  />}</Show>
                </Show>
              </section>
              <section class="candidate-review-section" aria-label="Decision">
                <h3>Before you accept</h3>
                <div class="candidate-review-decision-grid">
                  <div><span>Goes to</span><strong>{destinationLabel()}</strong></div>
                  <div><span>Saved version</span><strong>Version {Math.max(candidateVersion(), 1)}</strong><small>{prepared().verified ? "Saved copy is intact" : "Saved copy could not be checked"}</small></div>
                  <div><span>Format checks</span><strong>{prepared().draft_checks?.length ? `${prepared().draft_checks?.filter((check) => check.status === "passed").length} passed · ${prepared().draft_checks?.filter((check) => check.status !== "passed").length} failed on draft` : prepared().candidate.target_checks?.length ? `${prepared().candidate.target_checks?.length} planned` : "Unavailable"}</strong><small>{prepared().candidate.target_checks?.length ? "Draft checks inspect saved bytes. The same checks rerun after acceptance in the workspace." : "Vakyartha can't check this type of file automatically yet."}</small></div>
                </div>
                <For each={(prepared().draft_checks ?? []).filter((check) => check.status !== "passed")}>{(check) => <p role="alert" class="inline-error">A check failed on {check.path.split("/").pop()}.</p>}</For>
                <Show when={candidateComments().length > 0}>
                  <div class="candidate-review-comments" aria-label="Comments on this draft">
                    <h4>Comments</h4>
                    <For each={candidateComments()}>{(comment) => <article>
                      <div><strong>{comment.actor_id === "operator" ? "You" : comment.actor_name ?? comment.actor_id}</strong><span>{comment.path}{comment.anchor ? ` · ${comment.anchor}` : ""}{comment.line_start ? ` · line ${comment.line_start}${comment.line_end && comment.line_end !== comment.line_start ? `–${comment.line_end}` : ""}` : ""}</span></div>
                      <p>{comment.text}</p>
                      <button type="button" class="btn" disabled={reviewCommentBusy()} onClick={() => void requestRevisionFromComment(comment.comment_id)}>Ask Agent to address this</button>
                    </article>}</For>
                  </div>
                </Show>
                <div class="candidate-review-feedback">
                  <label for="candidate-review-comment">{commentAnchor() ? `Comment on ${commentAnchor()}` : "Comment on this draft"}</label>
                  <Show when={commentAnchor()}>
                    <button type="button" class="btn sm" onClick={() => setCommentAnchor(null)}>Comment on the whole file instead</button>
                  </Show>
                  <textarea id="candidate-review-comment" value={reviewComment()} onInput={(event) => setReviewComment(event.currentTarget.value)} placeholder="Describe what you want changed…" />
                  <button type="button" class="btn" disabled={reviewCommentBusy() || !reviewComment().trim()} onClick={() => void sendReviewComment()}>{reviewCommentBusy() ? "Saving…" : "Save comment"}</button>
                  <Show when={reviewCommentMessage()}>{(message) => <p role="status">{message()}</p>}</Show>
                </div>
                <Show when={technicalDetails()}>
                  <details class="candidate-review-provenance">
                    <summary>Technical details</summary>
                    <p><strong>Folder</strong> <span>{prepared().candidate.destination_root}</span></p>
                    <Show when={prepared().candidate.files.find((file) => file.path === reviewedPath())}>{(file) => <p><strong>File</strong> <span>{file().path} · {formatBytes(file().bytes)} · draft hash {file().candidate_hash.slice(0, 12)}</span></p>}</Show>
                    <p><strong>Run</strong> <span>{prepared().execution_id}</span></p>
                    <p><strong>Draft</strong> <span>{prepared().candidate.candidate_id}</span></p>
                    <p><strong>Result</strong> <span>{prepared().result_id}</span></p>
                    <p><strong>Digest</strong> <span>{prepared().candidate_digest}</span></p>
                  <Show when={(prepared().draft_checks?.length ?? 0) > 0}>
                    <div class="candidate-review-checks" aria-label="Saved draft format checks">
                      <strong>Saved draft checks</strong>
                      <For each={prepared().draft_checks}>{(check) => <p><strong>{check.status === "passed" ? "Passed" : "Failed"}</strong> · {check.path} · {check.evidence}</p>}</For>
                    </div>
                  </Show>
                  <Show when={(prepared().candidate.workspace_checks?.length ?? 0) > 0}>
                    <div class="candidate-review-checks">
                      <strong>Optional workspace checks after acceptance</strong>
                      <For each={prepared().candidate.workspace_checks}>{(check) => <p>{check.label} · <code>{check.command}</code></p>}</For>
                      <small>These commands run only when you choose Run workspace check after the files are accepted.</small>
                    </div>
                  </Show>
                  <Show when={previewPreparations().filter((record) => record.candidate_id === prepared().candidate.candidate_id).at(-1)}>{(record) =>
                    <div class="candidate-review-checks">
                      <h3>Preview environment</h3>
                      <p>{record().state} · <code>{record().command}</code></p>
                      <Show when={record().evidence}>
                        <details class="candidate-review-provenance">
                          <summary>Preparation details</summary>
                          <pre>{record().evidence.slice(-4_000)}</pre>
                        </details>
                      </Show>
                    </div>
                  }</Show>
                  </details>
                </Show>
              </section>
            </div>
        </Sheet>}
      </Show>
      <Show when={controlError()}>
        <div class="inline-error" role="alert">Could not stop this execution: {controlError()}</div>
      </Show>
      {/* Workbench Header */}
      <div class="workbench-header">
        <div class="workbench-header-right">
          <div class="workbench-nav-tabs" role="tablist" aria-label="Workbench views">
            <button
              class="workbench-nav-btn"
              role="tab"
              id="workbench-tab-execution"
              aria-controls="workbench-panel-execution"
              tabIndex={tab() === "execution" ? 0 : -1}
              aria-selected={tab() === "execution"}
              classList={{ active: tab() === "execution" }}
              onClick={() => setTab("execution")}
            >
              Activity<Show when={executions().length > 0}> ({executions().length})</Show>
            </button>
            <button
              class="workbench-nav-btn"
              role="tab"
              id="workbench-tab-artifacts"
              aria-controls="workbench-panel-artifacts"
              tabIndex={tab() === "artifacts" ? 0 : -1}
              aria-selected={tab() === "artifacts"}
              classList={{ active: tab() === "artifacts" }}
              onClick={() => setTab("artifacts")}
            >
              Files<Show when={allArtifacts().length > 0}> ({allArtifacts().length})</Show>
            </button>
          </div>
          <Show when={tab() === "execution" && executions().length > 0 && !technicalDetails()}><button type="button" class="workbench-detail-toggle" aria-pressed={showRunDetails()} onClick={() => setShowRunDetails((show) => !show)}>{showRunDetails() ? "Hide details" : "Technical details"}</button></Show>
          <Show when={tab() === "execution" && executions().length > 0}>
            <button
              class="workbench-clear-btn"
              onClick={clearExecutions}
              title="Clear activity history"
            >
              <Icon name="trash" size={14} />
            </button>
          </Show>
        </div>
      </div>
      <Show when={isAnyRunning()}>
        <div class="workbench-live-strip" aria-live="polite">
          <span class="pulse-dot" />
          <strong>Still working</strong>
          <span class="workbench-live-detail">
            {runningExec()?.stdout || runningExec()?.stderr ? "Receiving live output" : "Waiting for the next event"}
          </span>
          <span class="workbench-live-tick">{pulse() % 2 === 0 ? "·" : "…"}</span>
          <button class="btn" onClick={() => void stopExecution()} disabled={stopping()}>{stopping() ? "Stopping…" : "Stop"}</button>
        </div>
      </Show>

      {/* Main Content */}
      <Show when={executions().length === 0 && !selectedArtifact()}>
        <div class="workbench-empty-state">
          <Icon name={tab() === "artifacts" ? "file" : "history"} size={32} />
          <Show when={workbenchLoadError()}>
            <p class="error-state" role="alert">Couldn’t load this activity: {workbenchLoadError()}. Reopen this conversation to try again.</p>
          </Show>
          <p class="empty-title">{tab() === "artifacts" ? "No files yet" : "No activity yet"}</p>
          <p class="empty-desc">
            {tab() === "artifacts" ? "Files Vakyartha makes will appear here." : "You’ll see Vakyartha’s progress here when work begins."}
          </p>
        </div>
      </Show>

      <Show when={executions().length > 0 || selectedArtifact()}>
        <Show when={tab() === "execution"}>
          <div id="workbench-panel-execution" role="tabpanel" aria-labelledby="workbench-tab-execution" class="workbench-body" classList={{ "show-details": showRunDetails() || technicalDetails() }}>
            {/* Left list of runs */}
            <div class="workbench-runs-sidebar">
              <Index each={executions()}>
                {(item) => {
                  const isSelected = () => (currentExec()?.id ?? "") === item().id;
                  return (
                    <button
                      class="workbench-run-item"
                      aria-pressed={isSelected()}
                      aria-label={`Run ${item().timestamp}, ${item().status}${item().exitCode !== undefined ? `, exit code ${item().exitCode}` : ""}`}
                      classList={{
                        selected: isSelected(),
                        failed: item().status === "failed",
                        running: item().status === "running",
                      }}
                      onClick={() => setActiveExecutionId(item().id)}
                    >
                      <div class="run-item-header">
                        <span
                          class="status-indicator"
                          classList={{
                            running: item().status === "running",
                            success: item().status === "completed" && item().exitCode === 0,
                            failed:
                              item().status === "failed" ||
                              (item().exitCode !== undefined && item().exitCode !== 0),
                          }}
                        >
                          {item().status === "running" ? "●" : item().exitCode === 0 ? "✓" : "✗"}
                        </span>
                        <span class="run-time">{technicalDetails() ? item().timestamp : item().status === "running" ? "Working" : item().status === "failed" ? "Step failed" : "Completed step"}</span>
                        <Show when={technicalDetails() && item().durationMs !== undefined}>
                          <span class="run-duration">
                            {item().durationMs! < 1000
                              ? `${item().durationMs!}ms`
                              : `${(item().durationMs! / 1000).toFixed(1)}s`}
                          </span>
                        </Show>
                      </div>
                      <Show when={technicalDetails()}><div class="run-cmd-snippet" title={item().command}>{item().command}</div></Show>
                    </button>
                  );
                }}
              </Index>
            </div>

            {/* Execution Detail View */}
            <div class="workbench-run-detail">
              <Show when={showRunDetails() || technicalDetails()}>
              <Show when={currentExec()}>
                {(exec) => (
                  <div class="exec-detail-container">
                    {/* Command Banner */}
                    <div class="exec-banner">
                      <div class="exec-banner-top">
                        <div class="exec-cmd-info">
                          <span class="badge-lang">{exec().language}</span>
                          <span class="exec-tool-tag">{exec().tool}</span>
                          <Show when={exec().scratchDir}>
                            <span class="exec-cwd-tag" title={exec().scratchDir}>
                              {exec().scratchDir}
                            </span>
                          </Show>
                        </div>
                        <div class="exec-banner-actions">
                          <button
                            class="copy-btn"
                            onClick={() => copyCommand(exec().command)}
                            title="Copy command to clipboard"
                          >
                            <Icon name="copy" size={12} />
                            <span>{copiedCmd() ? "Copied!" : "Copy"}</span>
                          </button>
                        </div>
                      </div>
                      <pre class="exec-cmd-code"><code>{exec().command}</code></pre>
                    </div>

                    {/* Installed Packages Chips */}
                    <Show when={exec().packages.length > 0}>
                      <div class="exec-packages-card">
                        <span class="packages-label">Packages Installed:</span>
                        <div class="packages-chips">
                          <For each={exec().packages}>
                            {(pkg) => <span class="package-chip">{pkg}</span>}
                          </For>
                        </div>
                      </div>
                    </Show>

                    <Show when={exec().artifacts.length > 0 && !ranInScratch(exec())}>
                      <div class="exec-packages-card" title="These files are already in place; there is no draft to accept.">
                        <span class="packages-label">Written directly to the workspace</span>
                      </div>
                    </Show>

                    <Show when={exec().artifacts.length > 0 && ranInScratch(exec())}>
                      <div class="exec-packages-card">
                        <span class="packages-label">Workspace promotion</span>
                        <button class="tool-open" onClick={() => void reviewCandidate()} disabled={candidateBusy()}>
                          {candidateBusy() ? "Preparing review…" : "Review candidate"}
                        </button>
                        <Show when={candidate()}>
                          {(review) => (
                            <div style={{ "margin-top": "8px", width: "100%" }}>
                              <div class="artifact-meta">{review().candidate.files.length} file(s), hashed against the workspace base</div>
                              <button class="tool-open" onClick={() => setReviewOpen(true)}>Open file review</button>
                            </div>
                          )}
                        </Show>
                        <Show when={promotionMessage() ?? acceptanceMessage(exec().id)}>
                          {(message) => <div class="artifact-meta">{message()}</div>}
                        </Show>
                        <Show when={acceptanceFor(exec().id)}>
                          {(applied) => <div class="promotion-verification">
                            <Show when={applied().receipt.integration}>{(integration) => <>
                              <div class="artifact-meta"><strong>Workspace state verified</strong> · {integration().applied_state_digest.slice(0, 19)}</div>
                              <div class="artifact-meta">Target checks: {integration().target_checks_status}. {integration().target_checks_status === "unavailable" ? "Vakyartha had no check for these files in your folder." : integration().evidence}</div>
                              <For each={integration().target_checks ?? []}>{(check) => <div class="artifact-meta"><strong>{check.status === "passed" ? "Passed" : "Failed"}</strong> · {check.path} · {check.evidence}</div>}</For>
                            </>}</Show>
                            <For each={applied().workspace_checks ?? []}>{(check) => {
                              const latest = () => workspaceCheckReceipts(applied().candidate_id).filter((receipt) => receipt.check.id === check.id).at(-1);
                              return <div class="promotion-workspace-check">
                                <div class="artifact-meta"><strong>{check.label}</strong> · <code>{check.command}</code>{latest() ? ` · ${latest()?.status}` : ""}</div>
                                <Show when={latest()?.evidence}>{(evidence) => <div class="artifact-meta">{evidence()}</div>}</Show>
                                <button class="tool-open" disabled={!!workspaceCheckBusy()} onClick={() => void runWorkspaceCheck(applied(), check)}>{workspaceCheckBusy() === check.id ? "Running…" : latest() ? "Run again" : "Run workspace check"}</button>
                              </div>;
                            }}</For>
                            <button class="tool-open" disabled={undoBusy()} onClick={() => void undoPromotion(applied())}>
                              {undoBusy() ? "Restoring…" : "Undo acceptance"}
                            </button>
                          </div>}
                        </Show>
                      </div>
                    </Show>

                    {/* Terminal Stream Console */}
                    <div class="exec-terminal">
                      <div class="exec-terminal-header">
                        <span class="terminal-dot red" />
                        <span class="terminal-dot yellow" />
                        <span class="terminal-dot green" />
                        <span class="terminal-title">Terminal Stream</span>

                        {/* Live Telemetry Badges */}
                        <div style={{ display: "flex", "align-items": "center", gap: "8px", "margin-left": "auto" }}>
                          <Show when={exec().durationMs !== undefined}>
                            <span style={{ "font-size": "10.5px", color: "var(--muted)", "font-family": "monospace" }}>
                              ⏱ {exec().durationMs! < 1000
                                ? `${exec().durationMs}ms`
                                : `${(exec().durationMs! / 1000).toFixed(1)}s`}
                            </span>
                          </Show>

                          <Show when={exec().memoryBytes && exec().memoryBytes! > 0}>
                            <span style={{ "font-size": "10.5px", color: "var(--muted)", "font-family": "monospace" }}>
                              RAM: {formatBytes(exec().memoryBytes)}
                            </span>
                          </Show>

                          <Show when={exec().status === "running"}>
                            <button
                              onClick={stopExecution}
                              disabled={stopping()}
                              style={{
                                display: "inline-flex",
                                "align-items": "center",
                                gap: "4px",
                                background: "rgba(239, 68, 68, 0.2)",
                                color: "#f87171",
                                border: "1px solid rgba(239, 68, 68, 0.4)",
                                "border-radius": "4px",
                                padding: "2px 6px",
                                "font-size": "10.5px",
                                "font-weight": "600",
                                cursor: "pointer",
                              }}
                              title="Stop running command"
                            >
                              <Icon name="stop" size={10} />
                              <span>{stopping() ? "Stopping…" : "Stop"}</span>
                            </button>
                          </Show>

                          <button
                            class="copy-btn"
                            onClick={() => copyLog(exec())}
                            title="Copy full output log"
                          >
                            <Icon name="copy" size={11} />
                            <span>{copiedLog() ? "Copied!" : "Log"}</span>
                          </button>

                          <div class="terminal-status-tag">
                            <Show
                              when={exec().status === "running"}
                              fallback={
                                <span
                                  class="status-code"
                                  classList={{
                                    ok: exec().exitCode === 0,
                                    err: exec().exitCode !== 0,
                                  }}
                                >
                                  exit {exec().exitCode ?? 0}
                                </span>
                              }
                            >
                              <span class="status-running">Running…</span>
                            </Show>
                          </div>
                        </div>
                      </div>

                      <div class="exec-terminal-content" ref={terminalRef}>
                        <Show
                          when={exec().stdout || exec().stderr}
                          fallback={
                            <div class="terminal-idle">
                              {exec().status === "running" ? "Waiting for output…" : "(no output)"}
                            </div>
                          }
                        >
                          <Show when={exec().stdout}>
                            <pre
                              class="stdout-chunk"
                              innerHTML={renderAnsiToHtml(exec().stdout)}
                            />
                          </Show>
                          <Show when={exec().stderr}>
                            <pre
                              class="stderr-chunk"
                              innerHTML={renderAnsiToHtml(exec().stderr)}
                            />
                          </Show>
                          <Show when={exec().outputTruncated}>
                            <div class="terminal-truncated" role="status">
                              Output truncated after 1 MiB; the process continued safely.
                            </div>
                          </Show>
                        </Show>
                      </div>
                    </div>

                    {/* Artifacts generated in this run */}
                    <Show when={exec().artifacts.length > 0}>
                      <div class="exec-artifacts-section">
                        <div class="artifacts-title">Generated Artifacts:</div>
                        <div class="artifacts-grid">
                          <For each={exec().artifacts}>
                            {(art) => (
                              <div class="artifact-card-row">
                                <button
                                  class="artifact-card"
                                  onClick={() => inspectArtifact(art.path)}
                                >
                                  <Icon name="file" size={14} />
                                  <span class="artifact-path">{art.path}</span>
                                  <span class="artifact-meta">
                                    {art.mimeType} · {formatBytes(art.sizeBytes)}
                                  </span>
                                </button>
                                <button
                                  type="button"
                                  class="artifact-card-popout-btn"
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    openArtifactFile(art.path, runOrigin({ sessionId: exec().ownerSessionId ?? activeId() ?? undefined, executionId: exec().id }));
                                  }}
                                  title="Open in Artifact Canvas"
                                >
                                  <Icon name="preview" size={12} />
                                </button>
                              </div>
                            )}
                          </For>
                        </div>
                      </div>
                    </Show>
                  </div>
                )}
              </Show>
              </Show>
            </div>
          </div>
        </Show>

        {/* Artifacts Tab */}
        <Show when={tab() === "artifacts"}>
          <div id="workbench-panel-artifacts" role="tabpanel" aria-labelledby="workbench-tab-artifacts" class="workbench-artifacts-tab">
            <div class="result-workspace" classList={{ "has-selection": !!selectedArtifact() }}>
              <Show when={allArtifacts().length > 0}><div class="artifacts-list-sidebar">
              <Show
                when={allArtifacts().length > 0}
                  fallback={<div class="empty-list">Your finished files will appear here.</div>}
              >
                <For each={allArtifacts()}>
                  {(art) => {
                    const isSelected = () => selectedArtifact() === art.path;
                    return (
                      <div class="artifact-sidebar-item-row">
                        <button
                          class="artifact-sidebar-item"
                          aria-pressed={isSelected()}
                          aria-label={`Preview artifact ${art.path}`}
                          classList={{ selected: isSelected() }}
                          onClick={() => inspectArtifact(art.path)}
                        >
                          <Icon name="file" size={14} />
                          <div class="art-info">
                            <span class="art-name">{art.path.split("/").pop()}</span>
                            <span class="art-sub">
                              {technicalDetails() ? art.mimeType : (art.mimeType?.includes("wordprocessingml") ? "Word document" : art.mimeType?.includes("spreadsheetml") ? "Spreadsheet" : art.mimeType?.includes("presentationml") ? "Presentation" : art.mimeType?.includes("pdf") ? "PDF" : "File")} · {formatBytes(art.sizeBytes)}
                            </span>
                          </div>
                        </button>
                        <button
                          type="button"
                          class="artifact-popout-btn"
                          onClick={(e) => {
                            e.stopPropagation();
                            openArtifactFile(art.path, runOrigin({ sessionId: art.sessionId, executionId: art.executionId }));
                          }}
                          title="Open in Artifact Canvas"
                          aria-label={`Open ${art.path} in Artifact Canvas`}
                        >
                          <Icon name="preview" size={13} />
                        </button>
                      </div>
                    );
                  }}
                </For>
              </Show>
              </div>

              {/* Artifact Preview Viewer */}
              </Show><div class="artifact-viewer">
              <Show
                when={selectedArtifact()}
                fallback={
                  <div class="viewer-placeholder">
                  Select a result to open its preview.
                  </div>
                }
              >
                <div class="viewer-header">
                  <button type="button" class="viewer-back" onClick={() => { setBrowsingFiles(true); setSelectedArtifact(null); }}>← All files</button>
                  <span class="viewer-path">{technicalDetails() ? selectedArtifact() : selectedArtifact()?.split("/").pop()}</span>
                  <Show when={loadingArtifact()}>
                    <Skeleton kind="text" label="Loading the file" />
                  </Show>
                  <button
                    type="button"
                    class="pill-action-btn primary"
                    onClick={() => {
                      const p = selectedArtifact();
                      if (p) {
                        const source = artifactSource(p);
                        openArtifactFile(p, runOrigin({ sessionId: source?.sessionId, executionId: source?.executionId }));
                      }
                    }}
                    title="Open in the full viewer"
                    style={{ "margin-left": "auto" }}
                  >
                    <Icon name="preview" size={12} /> Open
                  </button>
                </div>
                <div class="viewer-content">
                  <Show when={artifactError()}>
                    <div class="viewer-error">{artifactError()}</div>
                  </Show>

                  {/* HTML Live Sandboxed Web Preview */}
                  <Show when={isHtmlArtifact(selectedArtifact()!) && artifactContent() !== null}>
                    <div style={{ width: "100%", height: "100%", "min-height": "400px" }}>
                      <iframe
                        srcdoc={artifactPreview()}
                        sandbox="allow-scripts"
                        style={{
                          width: "100%",
                          height: "100%",
                          "min-height": "400px",
                          border: "none",
                          background: "#ffffff",
                          "border-radius": "6px",
                        }}
                        title="Draft page preview"
                      />
                    </div>
                  </Show>

                  {/* Image Preview */}
                  <Show when={isImageArtifact(selectedArtifact()!) && artifactDataUrl()}>
                    <div class="image-preview" style={{ "text-align": "center", padding: "16px" }}>
                      <img
                        src={artifactDataUrl()!}
                        alt={selectedArtifact()!}
                        style={{ "max-width": "100%", "max-height": "500px", "border-radius": "4px" }}
                      />
                    </div>
                  </Show>

                  <Show when={isPdfArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType) && artifactDataUrl()}>
                    <iframe
                      src={artifactDataUrl()!}
                      title="PDF artifact preview"
                      style={{ width: "100%", height: "100%", "min-height": "520px", border: "none" }}
                    />
                  </Show>

                  <Show when={isAudioArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType) && artifactDataUrl()}>
                    <div class="media-preview"><audio src={artifactDataUrl()!} controls preload="metadata" /></div>
                  </Show>

                  <Show when={isVideoArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType) && artifactDataUrl()}>
                    <div class="media-preview"><video src={artifactDataUrl()!} controls preload="metadata" /></div>
                  </Show>

                  <Show when={isOfficePath(selectedArtifact()!)}>
                    <div class="artifact-office">
                      <OfficeView source={{ path: selectedArtifact()!, sessionId: artifactSource(selectedArtifact()!)?.sessionId, executionId: artifactSource(selectedArtifact()!)?.executionId }} fileName={selectedArtifact()!.split("/").pop() ?? selectedArtifact()!} />
                      <Show when={artifactDataUrl()}>{(url) => (
                        <a class="btn sm" href={url()} download={selectedArtifact()!.split("/").pop()}>Download file</a>
                      )}</Show>
                    </div>
                  </Show>

                  {/* Code / Text Preview */}
                  <Show
                    when={
                      !isOfficePath(selectedArtifact()!) &&
                      !isHtmlArtifact(selectedArtifact()!) &&
                      !isImageArtifact(selectedArtifact()!) &&
                      !isPdfArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType) &&
                      !isAudioArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType) &&
                      !isVideoArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType) &&
                      artifactContent() !== null
                    }
                  >
                    <pre class="code-preview"><code>{artifactContent()}</code></pre>
                  </Show>

                  <Show
                    when={
                      artifactDataUrl() !== null &&
                      !isOfficePath(selectedArtifact()!) &&
                      !isHtmlArtifact(selectedArtifact()!) &&
                      !isImageArtifact(selectedArtifact()!) &&
                      !isPdfArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType) &&
                      !isAudioArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType) &&
                      !isVideoArtifact(selectedArtifact()!, allArtifacts().find((a) => a.path === selectedArtifact())?.mimeType)
                    }
                  >
                    <div class="artifact-fallback">
                      <Icon name="file" size={24} />
                      <strong>This file is ready</strong>
                      <span>This format cannot be previewed here yet.</span>
                      <a class="btn primary sm" href={artifactDataUrl()!} download={selectedArtifact()!.split("/").pop()}>Download file</a>
                    </div>
                  </Show>
                </div>
              </Show>
              </div>
            </div>
          </div>
        </Show>
      </Show>
    </div>
  );
}
