import { createSignal, For, Show } from "solid-js";
import * as api from "../../api";
import { activeId, closeArtifactCanvas, updateCanvasEntry } from "../../store";
import { sendPrompt } from "../../App";
import { subjectPath, subjectSessionId } from "../../canvasSubject";
import type { CanvasEntry } from "../../canvasStack";
import type { DraftThread } from "./draftThread";

type SendState = "idle" | "sending" | "sent" | "saved_only" | "error";

/** Tells the Agent what to change in the subject; on a saved draft the note is a comment on that version. */
export default function CanvasFeedback(props: { entry: CanvasEntry; thread: DraftThread }) {
  const [state, setState] = createSignal<SendState>("idle");
  const subject = () => props.entry.subject;
  const key = () => props.entry.key;
  const draft = () => (subject().kind === "draft_file" ? subject() as Extract<ReturnType<typeof subject>, { kind: "draft_file" }> : undefined);
  const selection = () => props.entry.selection;
  const lineInvalid = () => {
    const range = selection();
    return !!range && range.end !== undefined && (range.start < 1 || range.end < range.start);
  };
  const number = (value: string) => {
    const parsed = Number.parseInt(value, 10);
    return Number.isFinite(parsed) ? parsed : undefined;
  };
  const setStart = (value: string) => {
    const start = number(value);
    updateCanvasEntry({ selection: start === undefined ? null : { start, end: selection()?.end } }, key());
  };
  const setEnd = (value: string) => {
    const range = selection();
    if (range) updateCanvasEntry({ selection: { start: range.start, end: number(value) } }, key());
  };
  const comments = () => props.thread.comments().filter((comment) => !comment.path || comment.path === subjectPath(subject()));

  const send = async () => {
    const current = subject();
    const sessionId = subjectSessionId(current) ?? activeId();
    const note = props.entry.draft.trim();
    if (!sessionId || !note || state() === "sending") return;
    setState("sending");
    let commentSaved = false;
    const label = subjectPath(current) || current.title;
    try {
      const version = draft();
      if (version) {
        const range = selection();
        const saved = await api.commentOnSandboxCandidate(sessionId, version.candidateId, note, {
          path: version.path || undefined,
          lineStart: range && range.start > 0 ? range.start : undefined,
          lineEnd: range?.end && range.end > 0 ? range.end : undefined,
        });
        commentSaved = true;
        updateCanvasEntry({ draft: "" }, key());
        void props.thread.refresh(version);
        await api.requestRevisionFromCandidateComment(sessionId, version.candidateId, saved.comment_id);
      } else {
        const result = current.resultId ? ` from result ${current.resultId}` : "";
        await sendPrompt(
          `Please revise the draft ${JSON.stringify(label)}${result}. Feedback: ${note}\nInspect the saved result and answer when the change is done; do not repeat a write when the file already contains the requested change.`,
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
      }
      updateCanvasEntry({ draft: "" }, key());
      setState("sent");
    } catch {
      setState(commentSaved ? "saved_only" : "error");
    }
  };

  return (
    <section class="artifact-canvas-feedback" aria-label="Review this draft">
      <div class="artifact-canvas-feedback-intro">
        <strong>Work on this together</strong>
        <span>Tell the Agent what to change in this draft.</span>
      </div>
      <div class="artifact-canvas-feedback-compose">
        <Show when={draft() && props.entry.view === "source"}>
          <div class="artifact-canvas-line-anchor" aria-label="Comment location">
            <label for="canvas-comment-line-start">Line</label>
            <input id="canvas-comment-line-start" type="number" min="1" inputmode="numeric" value={selection()?.start ?? ""} onInput={(event) => setStart(event.currentTarget.value)} placeholder="Start" />
            <span>to</span>
            <input type="number" min={selection()?.start ?? 1} inputmode="numeric" disabled={!selection()} value={selection()?.end ?? ""} onInput={(event) => setEnd(event.currentTarget.value)} placeholder="End" aria-label="End line" />
            <button type="button" class="artifact-canvas-btn" onClick={() => updateCanvasEntry({ selection: null }, key())}>Whole file</button>
          </div>
          <Show when={lineInvalid()}><small role="alert">End line must be on or after the start line.</small></Show>
        </Show>
        <textarea
          rows={2}
          value={props.entry.draft}
          onInput={(event) => { updateCanvasEntry({ draft: event.currentTarget.value }, key()); setState("idle"); }}
          onKeyDown={(event) => {
            if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
              event.preventDefault();
              void send();
            }
          }}
          placeholder="What should change?"
          aria-label="Feedback for this draft"
        />
        <button type="button" disabled={!props.entry.draft.trim() || state() === "sending" || lineInvalid()} onClick={() => void send()}>
          {state() === "sending" ? "Sending…" : "Ask for revision"}
        </button>
      </div>
      <Show when={state() === "sent"}><small role="status">Sent to the Agent conversation.</small></Show>
      <Show when={state() === "saved_only"}><small role="status">Comment saved. Open Review to ask the Agent to address it.</small></Show>
      <Show when={state() === "error"}><small role="alert">Could not send. Your feedback is still here to retry.</small></Show>
      <Show when={comments().length > 0}>
        <div class="artifact-canvas-comments" aria-label="Comments on this saved draft">
          <For each={comments()}>{(comment) =>
            <article>
              <div><strong>{comment.actor_id === "operator" ? "You" : comment.actor_name ?? comment.actor_id}</strong><span>{comment.path}{comment.line_start ? ` · line ${comment.line_start}${comment.line_end && comment.line_end !== comment.line_start ? `–${comment.line_end}` : ""}` : ""}</span></div>
              <p>{comment.text}</p>
            </article>
          }</For>
        </div>
      </Show>
    </section>
  );
}
