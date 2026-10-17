import { Show } from "solid-js";
import { presentationOf } from "../../store";
import PresentationTimelineView, { InCanvasContext } from "../PresentationRenderer";
import type { ViewerProps } from "./types";

/** A whole answer at full size (docs/design/98 §3): the turn drawn by the
 *  same view the conversation uses (its text, cards and files, in the
 *  model's order and placement), its parts flowing into as many columns as
 *  the Canvas fits. Everything works here as it does in the conversation:
 *  actions, and the values cards publish and follow, stay live. */
export default function AnswerViewer(props: ViewerProps) {
  const subject = () => (props.subject.kind === "answer" ? props.subject : null);
  const turn = () => {
    const answer = subject();
    const timeline = answer ? presentationOf(answer.sessionId) : null;
    if (!answer || !timeline) return null;
    const items = timeline.items.filter((item) => item.turn_id === answer.turnId);
    return items.length > 0 ? { ...timeline, items } : null;
  };
  return (
    <Show when={turn()} fallback={<p class="canvas-card-gone" role="status">This answer is no longer in the conversation.</p>}>
      {(timeline) => (
        <div class="answer-in-canvas">
          <InCanvasContext.Provider value={true}>
            <PresentationTimelineView timeline={timeline()} sessionId={subject()?.sessionId ?? ""} allowContinuation={false} hideUser />
          </InCanvasContext.Provider>
        </div>
      )}
    </Show>
  );
}
