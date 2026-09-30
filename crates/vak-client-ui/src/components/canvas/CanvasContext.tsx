import { createMemo, createSignal, For, Show } from "solid-js";
import * as api from "../../api";
import { activeId, closeArtifactCanvas, openArtifactCanvas, technicalDetails, updateCanvasEntry } from "../../store";
import { sendPrompt } from "../../App";
import { fileSubject, subjectPath, subjectSessionId } from "../../canvasSubject";
import type { CanvasEntry, ContextPanel } from "../../canvasStack";
import { commentPlace, selectionForPrompt, selectionInvalid, selectionLabel, selectionOfComment } from "../../canvasSelection";
import { viewerSpec } from "../../canvasViewers";
import { changedFiles, draftActivity } from "../../draftVersions";
import type { DraftSubject, DraftThread } from "./draftThread";

type SendState = "idle" | "sending" | "asked" | "commented" | "saved_only" | "error";

const CHANGE_WORDS = { new: "New", changed: "Changed", removed: "Removed" } as const;

/**
 * The area under the subject where people work on it together: the discussion
 * (comments about the place they have pointed at, and asking the Agent), what
 * has happened to the draft, and which files this version changes. The
 * comment on a saved draft belongs to that version.
 */
export default function CanvasContext(props: { entry: CanvasEntry; thread: DraftThread }) {
  const [state, setState] = createSignal<SendState>("idle");
  const subject = () => props.entry.subject;
  const key = () => props.entry.key;
  const draft = () => (subject().kind === "draft_file" ? (subject() as DraftSubject) : undefined);
  const spec = () => viewerSpec(subject());
  /** The selection this view can make; one made in another view is kept but not used. */
  const selection = createMemo(() => {
    const kind = spec().selects(props.entry.view);
    const current = props.entry.selection;
    return kind && current?.kind === kind ? current : null;
  });
  const lines = () => { const current = selection(); return current?.kind === "lines" ? current : null; };
  const invalid = () => selectionInvalid(selection());
  const number = (value: string) => {
    const parsed = Number.parseInt(value, 10);
    return Number.isFinite(parsed) ? parsed : undefined;
  };
  const setStart = (value: string) => {
    const start = number(value);
    updateCanvasEntry({ selection: start === undefined ? null : { kind: "lines", start, end: lines()?.end } }, key());
  };
  const setEnd = (value: string) => {
    const current = lines();
    if (current) updateCanvasEntry({ selection: { kind: "lines", start: current.start, end: number(value) } }, key());
  };
  const comments = () => props.thread.comments().filter((comment) => !comment.path || comment.path === subjectPath(subject()));
  const tabs = (): ContextPanel[] => (draft() ? ["discussion", "activity", "changes"] : ["discussion"]);
  const TAB_WORDS: Record<ContextPanel, string> = { discussion: "Discussion", activity: "Activity", changes: "Changes" };

  /** Puts the reader where a comment was written. */
  const goTo = (comment: api.SandboxCandidateComment) => {
    const place = selectionOfComment(comment);
    if (!place) return;
    const wantsSource = place.kind === "lines" && spec().selects(props.entry.view) !== "lines" && spec().views.some((choice) => choice.id === "source");
    updateCanvasEntry({ selection: place, ...(wantsSource ? { view: "source" } : {}) }, key());
  };

  const submit = async (askAgent: boolean) => {
    const current = subject();
    const sessionId = subjectSessionId(current) ?? activeId();
    const note = props.entry.draft.trim();
    if (!sessionId || !note || state() === "sending") return;
    setState("sending");
    let commentSaved = false;
    const label = subjectPath(current) || current.title;
    const place = commentPlace(selection());
    try {
      const version = draft();
      if (version) {
        const saved = await api.commentOnSandboxCandidate(sessionId, version.candidateId, note, { path: version.path || undefined, ...place });
        commentSaved = true;
        updateCanvasEntry({ draft: "" }, key());
        void props.thread.refresh(version);
        if (askAgent) await api.requestRevisionFromCandidateComment(sessionId, version.candidateId, saved.comment_id);
        setState(askAgent ? "asked" : "commented");
      } else {
        const result = current.resultId ? ` from result ${current.resultId}` : "";
        await sendPrompt(
          `Please revise the draft ${JSON.stringify(label)}${result}. Feedback: ${note}${selectionForPrompt(selection())}\nInspect the saved result and answer when the change is done; do not repeat a write when the file already contains the requested change.`,
          undefined,
          undefined,
          sessionId,
          { sessionId, resultId: current.resultId, label },
          "correction",
          true,
        );
        // A new Agent turn may raise a scoped approval. Return to the
        // conversation so its live controls and result are visible.
        closeArtifactCanvas();
        updateCanvasEntry({ draft: "" }, key());
        setState("asked");
      }
    } catch {
      setState(commentSaved ? "saved_only" : "error");
    }
  };

  const openFile = (path: string) => {
    const version = draft();
    if (!version) return;
    openArtifactCanvas(fileSubject(path, { sessionId: version.sessionId, candidateId: version.candidateId, executionId: version.executionId, resultId: version.resultId }, path.split("/").pop() || path));
  };
  const currentVersion = () => props.thread.versions().find((item) => item.candidate.candidate_id === draft()?.candidateId);
  const activity = () => draftActivity(props.thread.records(), props.thread.versions(), props.thread.comments());
  const when = (iso: string) => new Date(iso).toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });

  return (
    <section class="artifact-canvas-feedback" aria-label="Work on this together">
      <Show when={tabs().length > 1}>
        <div class="artifact-canvas-context-tabs" role="tablist" aria-label="About this draft">
          <For each={tabs()}>{(tab) =>
            <button type="button" role="tab" aria-selected={props.entry.panel === tab} classList={{ active: props.entry.panel === tab }} onClick={() => updateCanvasEntry({ panel: tab }, key())}>{TAB_WORDS[tab]}</button>
          }</For>
        </div>
      </Show>

      <Show when={props.entry.panel === "discussion" || tabs().length === 1}>
        <div class="artifact-canvas-feedback-intro">
          <strong>Work on this together</strong>
          <span>{selection() ? "Say what should change about the place you picked." : draft() ? "Comment on this version, or ask the Agent to change it." : "Tell the Agent what to change in this draft."}</span>
        </div>
        <div class="artifact-canvas-feedback-compose">
          <Show when={selection()}>{(picked) =>
            <div class="artifact-canvas-selection" role="status">
              <span>{selectionLabel(picked(), technicalDetails())}</span>
              <button type="button" class="artifact-canvas-btn" onClick={() => updateCanvasEntry({ selection: null }, key())}>Whole file</button>
            </div>
          }</Show>
          <Show when={draft() && spec().selects(props.entry.view) === "lines"}>
            <div class="artifact-canvas-line-anchor" aria-label="Comment location">
              <label for="canvas-comment-line-start">Line</label>
              <input id="canvas-comment-line-start" type="number" min="1" inputmode="numeric" value={lines()?.start ?? ""} onInput={(event) => setStart(event.currentTarget.value)} placeholder="Start" />
              <span>to</span>
              <input type="number" min={lines()?.start ?? 1} inputmode="numeric" disabled={!lines()} value={lines()?.end ?? ""} onInput={(event) => setEnd(event.currentTarget.value)} placeholder="End" aria-label="End line" />
            </div>
            <Show when={invalid()}><small role="alert">End line must be on or after the start line.</small></Show>
          </Show>
          <textarea
            rows={2}
            value={props.entry.draft}
            onInput={(event) => { updateCanvasEntry({ draft: event.currentTarget.value }, key()); setState("idle"); }}
            onKeyDown={(event) => {
              if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
                event.preventDefault();
                void submit(true);
              }
            }}
            placeholder="What should change?"
            aria-label="Feedback for this draft"
          />
          <div class="artifact-canvas-feedback-actions">
            <Show when={draft()}>
              <button type="button" class="artifact-canvas-btn" disabled={!props.entry.draft.trim() || state() === "sending" || invalid()} onClick={() => void submit(false)}>Comment</button>
            </Show>
            <button type="button" disabled={!props.entry.draft.trim() || state() === "sending" || invalid()} onClick={() => void submit(true)}>
              {state() === "sending" ? "Sending…" : draft() ? "Ask Agent" : "Ask for revision"}
            </button>
          </div>
        </div>
        <Show when={state() === "asked"}><small role="status">Sent to the Agent conversation.</small></Show>
        <Show when={state() === "commented"}><small role="status">Comment saved on this version.</small></Show>
        <Show when={state() === "saved_only"}><small role="status">Comment saved. Open Review to ask the Agent to address it.</small></Show>
        <Show when={state() === "error"}><small role="alert">Could not send. Your feedback is still here to retry.</small></Show>
        <Show when={comments().length > 0}>
          <div class="artifact-canvas-comments" aria-label="Comments on this saved draft">
            <For each={comments()}>{(comment) => {
              const place = selectionOfComment(comment);
              return <article>
                <div><strong>{comment.actor_id === "operator" ? "You" : comment.actor_name ?? comment.actor_id}</strong><span>{place ? selectionLabel(place, technicalDetails()) : "Whole file"}</span></div>
                <p>{comment.text}</p>
                <Show when={place}><button type="button" class="artifact-canvas-btn" onClick={() => goTo(comment)}>Show where</button></Show>
              </article>;
            }}</For>
          </div>
        </Show>
      </Show>

      <Show when={draft() && props.entry.panel === "activity"}>
        <ul class="artifact-canvas-activity" aria-label="What has happened to this draft">
          <For each={activity()} fallback={<li>Nothing yet.</li>}>{(item) => <li><span>{item.text}</span><time>{when(item.when)}</time></li>}</For>
        </ul>
      </Show>

      <Show when={draft() && props.entry.panel === "changes"}>
        <ul class="artifact-canvas-changes" aria-label="Files in this version">
          <For each={currentVersion() ? changedFiles(currentVersion()!) : []} fallback={<li>This version's files are not known yet.</li>}>{(file) =>
            <li>
              <span class="artifact-canvas-change" data-change={file.change}>{CHANGE_WORDS[file.change]}</span>
              <Show when={file.change !== "removed"} fallback={<span>{file.path}</span>}>
                <button type="button" class="artifact-canvas-btn" onClick={() => openFile(file.path)}>{file.path}</button>
              </Show>
            </li>
          }</For>
        </ul>
      </Show>
    </section>
  );
}
