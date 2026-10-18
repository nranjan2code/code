import { pageValuesKey } from "./AnswerValuesScope";
import PresentationTimelineView, { InCanvasContext } from "./PresentationRenderer";
import type { AnswerOutline, OutputItem } from "../types";

/** A page: an answer kept out of its conversation (docs/design/98 §12). */
export interface Page {
  format: "vak-page";
  title: string;
  items: OutputItem[];
  outline?: AnswerOutline | null;
  text: string;
}

/** The page these bytes hold, or `null` for anything else. */
export function readPage(bytes: Uint8Array): Page | null {
  try {
    const page = JSON.parse(new TextDecoder().decode(bytes)) as Page;
    return page?.format === "vak-page" && Array.isArray(page.items) ? page : null;
  } catch {
    return null;
  }
}

/** A page drawn by the view the conversation uses, so its cards, controls
 *  and shared values work as they did in the answer; a viewer's changes to
 *  its controls are their own and are never saved. Its parts flow into as
 *  many columns as there is room for. */
export default function PageView(props: { page: Page; versionId: string }) {
  const timeline = () => ({
    schema_version: 2,
    session_id: "",
    items: props.page.items,
    diagnostics: [],
    answers: props.page.outline ? [props.page.outline] : [],
  });
  return (
    <div class="answer-in-canvas">
      <InCanvasContext.Provider value={true}>
        <PresentationTimelineView timeline={timeline()} sessionId="" valueScope={pageValuesKey(props.versionId)} allowContinuation={false} hideUser />
      </InCanvasContext.Provider>
    </div>
  );
}
