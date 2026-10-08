import { createMemo, createSignal, For, Show } from "solid-js";
import * as api from "../../api";
import { activeId, canvasConversation, canvasMode, closeCanvasEntry, openArtifactCanvas, technicalDetails, updateCanvasEntry } from "../../store";
import { sendPrompt } from "../../App";
import { fileSubject, subjectPath, subjectSessionId } from "../../canvasSubject";
import type { CanvasEntry, ContextPanel } from "../../canvasStack";
import { changeRequest, commentPlace, selectionInvalid, selectionLabel, selectionOfComment } from "../../canvasSelection";
import { viewerSpec } from "../../canvasViewers";
import { changedFiles, draftActivity } from "../../draftVersions";
import type { DraftSubject, DraftThread } from "./draftThread";

type SendState = "idle" | "sending" | "asked" | "commented" | "saved_only" | "error";

const CHANGE_WORDS = { new: "New", changed: "Changed", removed: "Removed" } as const;

/**
 * The area under the subject where people work on it: the discussion
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
  const author = (comment: api.VersionComment) =>
    comment.actor_id === "operator" ? "You" : comment.actor_name ?? (technicalDetails() ? comment.actor_id : "Someone else");

  /** Puts the reader where a comment was written. */
  const goTo = (comment: api.VersionComment) => {
    const place = selectionOfComment(comment);
    if (!place) return;
    const wantsSource = place.kind === "lines" && spec().selects(props.entry.view) !== "lines" && spec().views.some((choice) => choice.id === "source");
    updateCanvasEntry({ selection: place, ...(wantsSource ? { view: "source" } : {}) }, key());
  };

  const submit = async (askAgent: boolean) => {
    // Everything this send changes belongs to the tab and conversation it was
    // sent from, even if the reader moves elsewhere while it is on its way.
    const conversation = canvasConversation();
    const tab = key();
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
        const at = props.thread.binding() ?? (await api.draftThread(sessionId, version.candidateId, version.path)).binding;
        if (!at) throw new Error("this draft has no version to comment on yet");
        const saved = await api.commentOnVersion(at.artifact, at.version, note, place);
        commentSaved = true;
        updateCanvasEntry({ draft: "" }, tab, conversation);
        void props.thread.refresh(version);
        if (askAgent) await api.reviseFromComment(at.artifact, saved.comment_id);
        setState(askAgent ? "asked" : "commented");
      } else {
        await sendPrompt(
          changeRequest(label, note, selection()),
          undefined,
          undefined,
          sessionId,
          { sessionId, resultId: current.resultId, label },
          "correction",
          true,
        );
        updateCanvasEntry({ draft: "" }, tab, conversation);
        setState("asked");
        // The Agent may ask before it acts. Beside the conversation that
        // question is already in view; on the whole window it would be
        // hidden, so the Canvas steps aside.
        if (conversation === canvasConversation() && (canvasMode() === "focused" || window.matchMedia("(max-width: 1100px)").matches)) closeCanvasEntry(tab, conversation);
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
  const unavailable = "This draft's history couldn't be loaded. It will try again when something changes.";

  return (
    <section class="artifact-canvas-feedback" aria-label={draft() ? "Work on this together" : "Ask for a change"}>
      <Show when={tabs().length > 1}>
        <div class="artifact-canvas-context-tabs" role="tablist" aria-label="About this draft">
          <For each={tabs()}>{(tab) =>
            <button type="button" role="tab" aria-selected={props.entry.panel === tab} classList={{ active: props.entry.panel === tab }} onClick={() => updateCanvasEntry({ panel: tab }, key())}>{TAB_WORDS[tab]}</button>
          }</For>
        </div>
      </Show>

      <Show when={props.entry.panel === "discussion" || tabs().length === 1}>
        <div class="artifact-canvas-feedback-intro">
          <strong>{draft() ? "Work on this together" : "Ask for a change"}</strong>
          <span>{selection() ? "Say what should change about the place you picked." : draft() ? "Comment on this version, or ask the Agent to change it." : "Tell the Agent what to change in this file."}</span>
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
            aria-label={draft() ? "Comment on this version" : "What should change in this file"}
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
        <Show when={state() === "asked"}><small role="status">Sent to the Agent. Its answer appears in the conversation.</small></Show>
        <Show when={state() === "commented"}><small role="status">Comment saved on this version.</small></Show>
        <Show when={state() === "saved_only"}><small role="status">Comment saved, but the Agent wasn't asked. Open Review to ask it to address the comment.</small></Show>
        <Show when={state() === "error"}><small role="alert">Couldn't send. What you wrote is still here, so you can try again.</small></Show>
        <Show when={comments().length > 0}>
          <div class="artifact-canvas-comments" aria-label="Comments on this saved draft">
            <For each={comments()}>{(comment) => {
              const place = selectionOfComment(comment);
              return <article>
                <div><strong>{author(comment)}</strong><span>{place ? selectionLabel(place, technicalDetails()) : "Whole file"}</span></div>
                <p>{comment.text}</p>
                <Show when={place}><button type="button" class="artifact-canvas-btn" onClick={() => goTo(comment)}>Show where</button></Show>
              </article>;
            }}</For>
          </div>
        </Show>
      </Show>

      <Show when={draft() && props.entry.panel === "activity"}>
        <ul class="artifact-canvas-activity" aria-label="What has happened to this draft">
          <For each={activity()} fallback={<li>{props.thread.unavailable() ? unavailable : "Nothing yet."}</li>}>{(item) => <li><span>{item.text}</span><time>{when(item.when)}</time></li>}</For>
        </ul>
      </Show>

      <Show when={draft() && props.entry.panel === "changes"}>
        <ul class="artifact-canvas-changes" aria-label="Files in this version">
          <For each={currentVersion() ? changedFiles(currentVersion()!) : []} fallback={<li>{props.thread.unavailable() ? unavailable : "This version's files are not known yet."}</li>}>{(file) =>
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
