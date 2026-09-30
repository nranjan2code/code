import type { CanvasSubject } from "../../canvasSubject";
import type { LineSelection } from "../../canvasStack";

/** What a viewer can do for the frame beyond drawing itself. */
export interface ViewerHandle {
  popout?: () => void;
}

/** Everything the frame gives a viewer. A viewer reads its own content. */
export interface ViewerProps {
  subject: CanvasSubject;
  /** Which of the viewer's views is showing (`ViewerSpec.views`). */
  view: string | null;
  /** Changes when the reader asks to reload. */
  reloadKey: number;
  selection: LineSelection | null;
  onSelect: (selection: LineSelection | null) => void;
  /** Present when the subject came from a review and can return to it. */
  onReview?: (candidateId?: string) => void;
  register: (handle: ViewerHandle | null) => void;
}
