import { Show } from "solid-js";
import { presentationOf } from "../../store";
import { StructuredView } from "../PresentationRenderer";
import { InCanvasContext } from "../cards/shared";
import type { ViewerProps } from "./types";

/** A card of the conversation at full size (docs/design/98 §3): the same
 *  form drawn by the same renderer, read from the conversation's
 *  presentation by its id, so its actions and the values it follows stay
 *  live. */
export default function CardViewer(props: ViewerProps) {
  const subject = () => (props.subject.kind === "card" ? props.subject : null);
  const item = () => {
    const card = subject();
    if (!card) return undefined;
    return presentationOf(card.sessionId)?.items.find((entry) =>
      entry.kind === "card" && entry.content.type === "structured" && entry.provenance?.presentation_id === card.presentationId);
  };
  return (
    <Show when={item()} fallback={<p class="canvas-card-gone" role="status">This card is no longer in the conversation.</p>}>
      {(entry) => (
        <div class="card-in-canvas">
          <InCanvasContext.Provider value={true}>
            <Show when={entry().content.type === "structured" && entry().content}>
              {(content) => content().type === "structured" && (
                <StructuredView output={(content() as { output: import("../../types").StructuredOutput }).output} fallback={entry().fallback_text}
                  sessionId={subject()?.sessionId} presentationId={subject()?.presentationId} cardState={entry().card_state} />
              )}
            </Show>
          </InCanvasContext.Provider>
        </div>
      )}
    </Show>
  );
}
