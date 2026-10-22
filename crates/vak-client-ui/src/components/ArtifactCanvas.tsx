import { canvasConflictReview, canvasSaveState, resolveCanvasConflict, retryCanvas } from "../canvasSync";
import { canvasViewport, observeCanvasViewport } from "../canvasDrawing";
import { CanvasWorkspaceContext, type CanvasWorkspace } from "./canvas/WorkspaceContext";
import { createWorkspaceEdits } from "../canvasWorkspace";
import { trapFocus } from "../focusTrap";
import ConfirmModal, { type ConfirmConfig } from "./ConfirmModal";
import CanvasResize from "./canvas/CanvasResize";
import CanvasSaveNotice from "./canvas/CanvasSaveNotice";
import { createEffect, createMemo, createSignal, ErrorBoundary, For, on, onCleanup, Show } from "solid-js";
import { Dynamic } from "solid-js/web";
import {
  activateCanvasEntry,
  activeId,
  canvasCloseRequests,
  canvasConversation,
  canvasDevice,
  canvasEntries,
  canvasEntry,
  canvasMode,
  canvasOpen,
  closeCanvasEntry,
  consumeCanvasArrival,
  openCandidateReview,
  setCanvasDevice,
  setNotice,
  technicalDetails,
  toggleCanvasMode,
  updateCanvasEntry,
} from "../store";
import { activate } from "../App";
import Icon from "./Icon";
import { subjectCandidateId, subjectExecutionId, subjectPath, subjectSessionId } from "../canvasSubject";
import { sameSelection, type Selection } from "../canvasSelection";
import { viewerSpec } from "../canvasViewers";
import CanvasContext from "./canvas/CanvasContext";
import NewVersionNotice from "./canvas/NewVersionNotice";
import { createDraftThread, type DraftSubject } from "./canvas/draftThread";
import type { ViewerHandle } from "./canvas/types";
import { VIEWERS } from "./canvas/viewers";

void trapFocus;

const DEVICES = [
  { id: "desktop", label: "Desktop", icon: "monitor" },
  { id: "tablet", label: "Tablet", icon: "tablet" },
  { id: "mobile", label: "Phone", icon: "phone" },
] as const;

const NARROW = "(max-width: 1100px)";
/** How long the Canvas takes to slide away (`canvasSlideOut`, `--dur`). */
const EXIT_MS = 220;

/**
 * ArtifactCanvas — the frame around whatever is open in the conversation's
 * Canvas: header, tabs, comment area and footer. What each kind of subject can
 * do is data (`canvasViewers.ts`) and how it is drawn is a viewer
 * (`canvas/viewers.ts`); the frame asks neither what kind it is.
 *
 * In "split" mode the conversation stays visible and usable beside it; in
 * "focused" mode it takes the full width. Each conversation has its own
 * Canvas, the same on every surface showing the conversation: switching away
 * hides it, coming back (here or in the other app) finds the same tabs with
 * the same view, selection and unsent note.
 *
 * Design-61 compliant: never auto-opens. Only appears on explicit user action.
 */
export default function ArtifactCanvas() {
  /** The tab sliding away: the conversation it belongs to, so switching
   *  conversation part way through still closes it. */
  const [confirmClose, setConfirmClose] = createSignal<ConfirmConfig | null>(null);
  const [closing, setClosing] = createSignal<{ conversation: string; key: string } | null>(null);
  const [arriving, setArriving] = createSignal(false);
  const [reloadKey, setReloadKey] = createSignal(0);
  const [handle, setHandle] = createSignal<ViewerHandle | null>(null);
  const query = window.matchMedia(NARROW);
  const [narrow, setNarrow] = createSignal(query.matches);
  const effectiveMode = () => narrow() ? "focused" : canvasMode();
  let closeTimeout: ReturnType<typeof setTimeout> | undefined;
  let arrivalTimeout: ReturnType<typeof setTimeout> | undefined;

  const onNarrow = (event: MediaQueryListEvent) => setNarrow(event.matches);
  query.addEventListener("change", onNarrow);
  onCleanup(() => {
    query.removeEventListener("change", onNarrow);
    if (closeTimeout) clearTimeout(closeTimeout);
    if (arrivalTimeout) clearTimeout(arrivalTimeout);
  });

  const entryKey = createMemo(() => canvasEntry()?.key);
  const entryIdentity = createMemo(() => entryKey() ? JSON.stringify([canvasConversation(), entryKey()]) : undefined);
  const subject = () => canvasEntry()?.subject;
  const spec = () => { const current = subject(); return current ? viewerSpec(current) : undefined; };
  const draft = createMemo(() => {
    const current = subject();
    return current?.kind === "draft_file" ? (current as DraftSubject) : undefined;
  });
  const thread = createDraftThread(draft);
  const title = () => subject()?.title || "Preview";
  const path = () => { const current = subject(); return current ? subjectPath(current) : ""; };
  const view = () => canvasEntry()?.view ?? null;
  const showsPage = () => !!spec()?.devices && view() !== "source";

  /** Ends the close now: the tab goes, in the conversation it belongs to. */
  const finishClose = () => {
    const leaving = closing();
    if (closeTimeout) clearTimeout(closeTimeout);
    closeTimeout = undefined;
    setClosing(null);
    if (leaving) closeCanvasEntry(leaving.key, leaving.conversation);
  };

  // Another tab in front, or another conversation, ends a close that was on
  // its way out: the tab that was asked to close still closes.
  createEffect(on([entryKey, canvasConversation], () => {
    const leaving = closing();
    if (leaving && (leaving.conversation !== canvasConversation() || leaving.key !== entryKey())) finishClose();
    setHandle(null);
  }, { defer: true }));

  // Keep the external opener across conversation/view remounts. A replacement
  // dialog can otherwise capture a header control from the departing Canvas,
  // leaving no connected element to restore when the final Canvas closes.
  let externalOpener: HTMLElement | null = null;
  createEffect(on(canvasOpen, open => {
    if (open) {
      const active = document.activeElement;
      if (active instanceof HTMLElement && active !== document.body && !active.closest(".artifact-canvas, [role='dialog']")) externalOpener = active;
    } else {
      queueMicrotask(() => {
        const active = document.activeElement;
        if (!canvasOpen() && externalOpener?.isConnected && (active === document.body || (active instanceof HTMLElement && active.closest(".artifact-canvas")))) externalOpener.focus({ preventScroll: true });
      });
    }
  }));

  // A Canvas a reader opened slides in; one coming back with its conversation,
  // or opened on another surface, is simply there.
  createEffect(on(canvasOpen, (open) => {
    if (!open) return;
    const arrival = consumeCanvasArrival();
    setArriving(arrival);
    if (arrivalTimeout) clearTimeout(arrivalTimeout);
    if (arrival) arrivalTimeout = setTimeout(() => setArriving(false), 400);
  }));

  const requestClose = (key: string, conversation = canvasConversation()) => {
    const entry = canvasEntries().find(item => item.key === key);
    if (Object.keys(entry?.checkpoints ?? {}).length) {
      setConfirmClose({ title: "Close with unsaved edits?", description: "Save your work in this workspace before closing, or discard these edits.",
        confirmLabel: "Discard edits and close", cancelLabel: "Keep working", isDanger: true,
        onConfirm: () => closeCanvasEntry(key, conversation) });
      return false;
    }
    closeCanvasEntry(key, conversation);
    return true;
  };

  const handleClose = () => {
    const key = entryKey();
    if (closing() || !key) return;
    // Another tab comes forward at once; only the last one slides away.
    if (Object.keys(canvasEntry()?.checkpoints ?? {}).length || canvasEntries().length > 1) {
      requestClose(key);
      return;
    }
    setClosing({ conversation: canvasConversation(), key });
    closeTimeout = setTimeout(finishClose, EXIT_MS);
  };

  // Escape is the app's (App.tsx): it asks, after menus and dialogs have had it.
  createEffect(on(canvasCloseRequests, () => handleClose(), { defer: true }));

  const returnToReview = (candidate?: string) => {
    const current = subject();
    const key = entryKey();
    const run = current && subjectExecutionId(current);
    if (!current || !run || !key) return;
    const version = candidate ?? subjectCandidateId(current);
    const session = subjectSessionId(current);
    closeCanvasEntry(key);
    openCandidateReview(run, session, version);
  };
  const returnToConversation = () => {
    const current = subject();
    const key = entryKey();
    const sessionId = current && subjectSessionId(current);
    if (key) closeCanvasEntry(key);
    if (sessionId && sessionId !== activeId()) void activate(sessionId);
  };

  return (
    <Show when={canvasOpen()}>
      {/* Backdrop: pointer-events: none in split mode to keep chat completely interactive; active in focused mode */}
      <div
        class="artifact-canvas-backdrop"
        classList={{
          focused: effectiveMode() === "focused",
          "canvas-arriving": arriving(),
          "canvas-closing": !!closing(),
        }}
        onClick={(e) => {
          if (e.target === e.currentTarget && effectiveMode() === "focused") handleClose();
        }}
        role="presentation"
      />

      {/* Canvas panel */}
      <div
        class="artifact-canvas"
        classList={{
          "canvas-split": effectiveMode() === "split",
          "canvas-focused": effectiveMode() === "focused",
          "canvas-arriving": arriving(),
          "canvas-closing": !!closing(),
        }}
        role="dialog"
        aria-label={`Canvas: ${title()}`}
        aria-modal={effectiveMode() === "focused" || narrow() ? "true" : "false"}
        use:trapFocus={effectiveMode() === "focused" || narrow()}
      >
        <Show when={!narrow() && effectiveMode() === "split"}><CanvasResize /></Show>
        <ConfirmModal config={confirmClose()} onClose={() => setConfirmClose(null)} />
        {/* Title bar */}
        <header class="artifact-canvas-header" data-titlebar>
          <div class="artifact-canvas-title-group">
            <span class="artifact-canvas-badge">
              {draft() ? `Draft preview${thread.version() ? ` · Version ${thread.version()}` : ""}` : spec()?.badge}
            </span>
            <strong class="artifact-canvas-title">{title()}</strong>
            <Show when={path() && technicalDetails()}>
              <span class="artifact-canvas-path">{path()}</span>
            </Show>
            <Show when={subject()?.resultId}>{(resultId) =>
              <button type="button" class="artifact-canvas-result" title={technicalDetails() ? resultId() : undefined} onClick={returnToConversation}>Back to the answer</button>
            }</Show>
          </div>

          <div class="artifact-canvas-controls">
            <Show when={(spec()?.views.length ?? 0) > 1}>
              <div class="artifact-canvas-segmented">
                <For each={spec()?.views}>{(choice) =>
                  <button
                    type="button"
                    class="artifact-canvas-seg-btn"
                    classList={{ active: view() === choice.id }}
                    onClick={() => updateCanvasEntry({ view: choice.id })}
                    title={choice.title}
                    aria-pressed={view() === choice.id}
                  >
                    {choice.label}
                  </button>
                }</For>
              </div>
            </Show>

            {/* Page widths. Kept in place while the code shows, so the
                switch beside it never moves under the pointer. */}
            <Show when={spec()?.devices}>
              <div class="artifact-canvas-device-group" classList={{ resting: !showsPage() }} aria-hidden={showsPage() ? undefined : "true"}>
                <For each={DEVICES}>{(device) =>
                  <button
                    type="button"
                    class="artifact-canvas-device-btn"
                    classList={{ active: canvasDevice() === device.id }}
                    onClick={() => setCanvasDevice(device.id)}
                    title={device.label}
                    aria-label={device.label}
                    aria-pressed={canvasDevice() === device.id}
                    disabled={!showsPage()}
                    tabIndex={showsPage() ? undefined : -1}
                  >
                    <Icon name={device.icon} size={16} />
                  </button>
                }</For>
              </div>
            </Show>

            <Show when={spec()?.reloadable}>
              <button type="button" class="artifact-canvas-btn" onClick={() => setReloadKey((key) => key + 1)} title="Reload" aria-label="Reload"><Icon name="sync" size={14} /></button>
            </Show>
            <Show when={!narrow()}>
              <button type="button" class="artifact-canvas-btn" onClick={toggleCanvasMode} title={effectiveMode() === "split" ? "Expand to full width" : "Show beside conversation"} aria-label={effectiveMode() === "split" ? "Expand to full width" : "Show beside conversation"}><Icon name={effectiveMode() === "split" ? "layers" : "restore"} size={14} /></button>
            </Show>
            <Show when={spec()?.popout && handle()?.popout}>{(popout) =>
              <button type="button" class="artifact-canvas-btn" onClick={popout()} title="Open in new window" aria-label="Open in new window"><Icon name="preview" size={14} /></button>
            }</Show>
            <Show when={subject() && subjectCandidateId(subject()!) && subjectExecutionId(subject()!)}>
              <button type="button" class="artifact-canvas-btn" onClick={() => returnToReview()} title="Back to review">
                <Icon name="diff" size={14} /> Review changes
              </button>
            </Show>
            <Show when={spec()?.feedback !== "none"}>
              <button type="button" class="artifact-canvas-btn" aria-expanded={!!canvasEntry()?.feedbackOpen} onClick={() => updateCanvasEntry({ feedbackOpen: !canvasEntry()?.feedbackOpen })}>{canvasEntry()?.feedbackOpen ? "Hide comments" : "Comment"}</button>
            </Show>
            <button
              type="button"
              class="artifact-canvas-close"
              onClick={handleClose}
              title="Close (Esc)"
              aria-label="Close Canvas"
            >
              <Icon name="close" size={16} />
            </button>
          </div>
        </header>

        <Show when={canvasEntries().length > 1}>
          <div class="artifact-canvas-tabs" role="tablist" aria-label="Open in this conversation">
            <For each={canvasEntries()}>{(item) =>
              <div class="artifact-canvas-tab" classList={{ active: item.key === entryKey() }}>
                <button type="button" role="tab" aria-selected={item.key === entryKey()} class="artifact-canvas-tab-label" tabIndex={item.key === entryKey() ? 0 : -1} onKeyDown={event => {
                  if (event.key === "Delete") { event.preventDefault(); requestClose(item.key); return; }
                  if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
                  event.preventDefault();
                  const tabs = canvasEntries();
                  const at = tabs.findIndex(tab => tab.key === item.key);
                  const index = event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1 : (at + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
                  activateCanvasEntry(tabs[index].key);
                  (event.currentTarget.closest('[role="tablist"]')?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[index])?.focus();
                }} title={technicalDetails() ? subjectPath(item.subject) || item.subject.title : item.subject.title} onClick={() => activateCanvasEntry(item.key)}>{item.subject.title}<Show when={Object.keys(item.checkpoints ?? {}).length}><span class="canvas-unsaved" aria-label="Unsaved edits"> ·</span></Show></button>
                <button type="button" class="artifact-canvas-tab-close" aria-label={`Close ${item.subject.title}`} onClick={() => requestClose(item.key)}><Icon name="close" size={12} /></button>
              </div>
            }</For>
          </div>
        </Show>

        <CanvasSaveNotice status={canvasSaveState(canvasConversation())} conflict={canvasConflictReview(canvasConversation())} onResolve={(token, choices) => resolveCanvasConflict(canvasConversation(), token, choices)} onRetry={() => retryCanvas(canvasConversation())} onCopy={async () => {
          const notes = canvasEntries().filter((entry) => entry.draft.length > 0 || Object.keys(entry.checkpoints ?? {}).length > 0)
            .map((entry) => `${entry.subject.title}\n${entry.draft}\n${Object.values(entry.checkpoints ?? {}).map(item => item.summary).join("\n")}`).join("\n\n");
          try {
            await navigator.clipboard.writeText(notes);
            setNotice({ kind: "info", text: "Unsaved Canvas work copied." });
          } catch {
            setNotice({ kind: "error", text: "Could not copy. Select and copy your unsaved work before closing." });
          }
        }} />

        <Show when={draft()}>{(version) =>
          <Show when={canvasEntry()}>{(entry) =>
            <NewVersionNotice entry={entry()} draft={version()} thread={thread} onReview={(candidateId) => returnToReview(candidateId)} />
          }</Show>
        }</Show>

        {/* Keyed by identity: another tab in front is a new viewer, with its own load and error state. */}
        <Show when={entryIdentity()} keyed>{(_identity) => {
          const key = entryKey()!;
          const conversation = canvasConversation();
          let last = canvasEntry()!;
          const current = () => {
            const found = canvasEntry();
            if (canvasConversation() === conversation && found?.key === key) last = found;
            return last;
          };
          // What each viewer is handed changes only when it changes: a reader's
          // selection or unsent note must not look like a new subject.
          const subjectNow = createMemo(() => current().subject);
          const viewNow = createMemo(() => current().view);
          const selectionNow = createMemo(() => current().selection, undefined, { equals: sameSelection });
          const [viewport, setViewport] = createSignal(canvasViewport(0, 0, window.devicePixelRatio));
          const stage = (element: HTMLDivElement) => {
            const dispose = observeCanvasViewport(element, setViewport);
            onCleanup(dispose);
          };
          const edits = createWorkspaceEdits(() => current().checkpoints,
            checkpoints => updateCanvasEntry({ checkpoints }, key, conversation));
          onCleanup(edits.dispose);
          const workspace: CanvasWorkspace = { viewport, signal: edits.signal,
            read: edits.read, checkpoint: edits.checkpoint, clear: edits.clear };

          return (
            <>
              <div class="artifact-canvas-body" ref={stage}>
                <ErrorBoundary fallback={(error, reset) =>
                  <div class="artifact-canvas-error" role="alert">
                    <Icon name="warning" size={16} />
                    <span>This view stopped working.<Show when={technicalDetails() && error instanceof Error && error.message}> {(error as Error).message}</Show></span>
                    <button type="button" class="artifact-canvas-btn" onClick={reset}>Try again</button>
                    <button type="button" class="artifact-canvas-btn" onClick={handleClose}>Close</button>
                  </div>
                }>
                  <CanvasWorkspaceContext.Provider value={workspace}><Dynamic
                    component={VIEWERS[viewerSpec(subjectNow()).type]}
                    subject={subjectNow()}
                    view={viewNow()}
                    reloadKey={reloadKey()}
                    selection={selectionNow()}
                    onSelect={(selection: Selection | null) => { if (!edits.signal.aborted) updateCanvasEntry(selection ? { selection, feedbackOpen: true, panel: "discussion" } : { selection }, key, conversation); }}
                    onReview={(candidateId: string | undefined) => { if (!edits.signal.aborted) returnToReview(candidateId); }}
                    register={(handle: ViewerHandle | null) => { if (!edits.signal.aborted) setHandle(handle); }}
                  /></CanvasWorkspaceContext.Provider>
                </ErrorBoundary>
              </div>
              <Show when={current().feedbackOpen}>
                <CanvasContext entry={current()} thread={thread} />
              </Show>
            </>
          );
        }}</Show>

        {/* What the frame is allowed to do */}
        <Show when={spec()?.safetyFooter}><footer class="artifact-canvas-footer">
          <span class="artifact-canvas-security">
            <Icon name="shield" size={12} />
            {handle()?.safetyLabel ?? (subject()?.kind === "live_server" ? "Live preview, running on this computer" : "Safe preview, offline")}
          </span>
        </footer></Show>
      </div>
    </Show>
  );
}
