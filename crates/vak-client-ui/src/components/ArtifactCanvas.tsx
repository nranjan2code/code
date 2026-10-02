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
  const [closing, setClosing] = createSignal<{ conversation: string; key: string } | null>(null);
  const [arriving, setArriving] = createSignal(false);
  const [reloadKey, setReloadKey] = createSignal(0);
  const [handle, setHandle] = createSignal<ViewerHandle | null>(null);
  const query = window.matchMedia(NARROW);
  const [narrow, setNarrow] = createSignal(query.matches);
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

  // A Canvas a reader opened slides in; one coming back with its conversation,
  // or opened on another surface, is simply there.
  createEffect(on(canvasOpen, (open) => {
    if (!open) return;
    const arrival = consumeCanvasArrival();
    setArriving(arrival);
    if (arrivalTimeout) clearTimeout(arrivalTimeout);
    if (arrival) arrivalTimeout = setTimeout(() => setArriving(false), 400);
  }));

  const handleClose = () => {
    const key = entryKey();
    if (closing() || !key) return;
    // Another tab comes forward at once; only the last one slides away.
    if (canvasEntries().length > 1) {
      closeCanvasEntry(key);
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
          focused: canvasMode() === "focused",
          "canvas-arriving": arriving(),
          "canvas-closing": !!closing(),
        }}
        onClick={(e) => {
          if (e.target === e.currentTarget && canvasMode() === "focused") handleClose();
        }}
        role="presentation"
      />

      {/* Canvas panel */}
      <div
        class="artifact-canvas"
        classList={{
          "canvas-split": canvasMode() === "split",
          "canvas-focused": canvasMode() === "focused",
          "canvas-arriving": arriving(),
          "canvas-closing": !!closing(),
        }}
        role="dialog"
        aria-label={`Canvas: ${title()}`}
        aria-modal={canvasMode() === "focused" ? "true" : "false"}
      >
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
              <button type="button" class="artifact-canvas-btn" onClick={toggleCanvasMode} title={canvasMode() === "split" ? "Expand to full width" : "Show beside conversation"} aria-label={canvasMode() === "split" ? "Expand to full width" : "Show beside conversation"}><Icon name={canvasMode() === "split" ? "layers" : "restore"} size={14} /></button>
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
                <button type="button" role="tab" aria-selected={item.key === entryKey()} class="artifact-canvas-tab-label" title={technicalDetails() ? subjectPath(item.subject) || item.subject.title : item.subject.title} onClick={() => activateCanvasEntry(item.key)}>{item.subject.title}</button>
                <button type="button" class="artifact-canvas-tab-close" aria-label={`Close ${item.subject.title}`} onClick={() => closeCanvasEntry(item.key)}><Icon name="close" size={12} /></button>
              </div>
            }</For>
          </div>
        </Show>

        <Show when={draft()}>{(version) =>
          <Show when={canvasEntry()}>{(entry) =>
            <NewVersionNotice entry={entry()} draft={version()} thread={thread} onReview={(candidateId) => returnToReview(candidateId)} />
          }</Show>
        }</Show>

        {/* Keyed by identity: another tab in front is a new viewer, with its own load and error state. */}
        <Show when={entryKey()} keyed>{(key) => {
          let last = canvasEntry()!;
          const current = () => {
            const found = canvasEntry();
            if (found?.key === key) last = found;
            return last;
          };
          // What each viewer is handed changes only when it changes: a reader's
          // selection or unsent note must not look like a new subject.
          const subjectNow = createMemo(() => current().subject);
          const viewNow = createMemo(() => current().view);
          const selectionNow = createMemo(() => current().selection, undefined, { equals: sameSelection });
          return (
            <>
              <div class="artifact-canvas-body">
                <ErrorBoundary fallback={(error, reset) =>
                  <div class="artifact-canvas-error" role="alert">
                    <Icon name="warning" size={16} />
                    <span>This view stopped working.<Show when={technicalDetails() && error instanceof Error && error.message}> {(error as Error).message}</Show></span>
                    <button type="button" class="artifact-canvas-btn" onClick={reset}>Try again</button>
                    <button type="button" class="artifact-canvas-btn" onClick={handleClose}>Close</button>
                  </div>
                }>
                  <Dynamic
                    component={VIEWERS[viewerSpec(subjectNow()).type]}
                    subject={subjectNow()}
                    view={viewNow()}
                    reloadKey={reloadKey()}
                    selection={selectionNow()}
                    onSelect={(selection: Selection | null) => updateCanvasEntry(selection ? { selection, feedbackOpen: true, panel: "discussion" } : { selection }, key)}
                    onReview={returnToReview}
                    register={setHandle}
                  />
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
            {subject()?.kind === "live_server" ? "Live preview, running on this computer" : "Safe preview, offline"}
          </span>
        </footer></Show>
      </div>
    </Show>
  );
}
