import { Show } from "solid-js";
import { PdfPages } from "./MediaViewer";
import OfficeViewer from "./OfficeViewer";
import type { ViewerProps } from "./types";

/** A PDF's own pages, or its text in the shared document pane. */
export default function PdfViewer(props: ViewerProps) {
  return (
    <Show when={props.view === "text"} fallback={<PdfPages {...props} />}>
      <OfficeViewer {...props} />
    </Show>
  );
}
