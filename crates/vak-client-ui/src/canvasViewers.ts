// What each kind of viewer can do, as data. The Canvas frame draws its header,
// tabs and comment area from this and never asks a viewer's kind directly, so
// a new subject kind is a registry entry plus a viewer component
// (docs/plans/canvas-plan.md K2).

import { displayType, type ArtifactDisplayType, type CanvasSubject } from "./canvasSubject.ts";
import type { CanvasMode } from "./canvasStack.ts";
import type { SelectionKind } from "./canvasSelection.ts";

export interface ViewChoice {
  id: string;
  label: string;
  title: string;
}

export interface ViewerSpec {
  type: ArtifactDisplayType;
  /** The short label in front of the title. */
  badge: string;
  /** Ways to look at the same subject; more than one draws a switch. */
  views: readonly ViewChoice[];
  /** The view shown first, on a wide screen and on a phone. */
  defaultView: (narrow: boolean) => string | null;
  /** Where it opens on a wide screen. Either can be chosen afterwards. */
  layout: CanvasMode;
  reloadable: boolean;
  /** Can be shown at desktop, tablet and phone widths. */
  devices: boolean;
  /** Can be opened in a window of its own. */
  popout: boolean;
  /** Whether the comment area is open when the subject opens or only on request. */
  feedback: "always" | "on_request";
  /** What a reader can point at in the given view, or nothing: a preview page or an image has no place a comment could name. */
  selects: (view: string | null) => SelectionKind | null;
  /** The footer that says what the frame is allowed to do. */
  safetyFooter: boolean;
}

const PREVIEW_OR_CODE: readonly ViewChoice[] = [
  { id: "preview", label: "Preview", title: "View interactive preview" },
  { id: "source", label: "Code", title: "View source text" },
];

const SPECS: Record<ArtifactDisplayType, ViewerSpec> = {
  html: { type: "html", badge: "Preview", views: PREVIEW_OR_CODE, defaultView: () => "preview", layout: "split", reloadable: true, devices: true, popout: true, feedback: "always", selects: (view) => (view === "source" ? "lines" : null), safetyFooter: true },
  server: { type: "server", badge: "Live preview", views: [], defaultView: () => null, layout: "split", reloadable: true, devices: true, popout: true, feedback: "always", selects: () => null, safetyFooter: true },
  code: { type: "code", badge: "Preview", views: [], defaultView: () => null, layout: "split", reloadable: false, devices: false, popout: false, feedback: "always", selects: () => "lines", safetyFooter: true },
  table: { type: "table", badge: "Data", views: [{ ...PREVIEW_OR_CODE[0], title: "View data table" }, PREVIEW_OR_CODE[1]], defaultView: () => "preview", layout: "split", reloadable: false, devices: false, popout: false, feedback: "always", selects: (view) => (view === "source" ? "lines" : null), safetyFooter: true },
  image: { type: "image", badge: "Image", views: [], defaultView: () => null, layout: "split", reloadable: false, devices: false, popout: false, feedback: "always", selects: () => null, safetyFooter: true },
  pdf: {
    type: "pdf",
    badge: "PDF",
    views: [{ id: "pages", label: "Pages", title: "View the pages" }, { id: "text", label: "Text", title: "Read, comment on and edit the text" }],
    // A phone reads the text; the pages stay one tap away.
    defaultView: (narrow) => (narrow ? "text" : "pages"),
    layout: "focused",
    reloadable: false,
    devices: false,
    popout: false,
    feedback: "on_request",
    selects: (view) => (view === "text" ? "anchor" : null),
    safetyFooter: false,
  },
  office: { type: "office", badge: "Document", views: [], defaultView: () => null, layout: "focused", reloadable: false, devices: false, popout: false, feedback: "on_request", selects: () => "anchor", safetyFooter: false },
};

export function viewerSpec(subject: CanvasSubject): ViewerSpec {
  return SPECS[displayType(subject)];
}

/** The state of a subject that was not open yet. */
export function freshEntry(subject: CanvasSubject, narrow: boolean) {
  const spec = viewerSpec(subject);
  return {
    // A phone has no room to put a conversation beside a document.
    mode: narrow ? ("focused" as const) : spec.layout,
    view: spec.defaultView(narrow),
    selection: null,
    draft: "",
    feedbackOpen: spec.feedback === "always",
    panel: "discussion" as const,
    dismissedVersion: 0,
  };
}
