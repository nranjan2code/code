import { Show } from "solid-js";
import { PdfPages } from "./MediaViewer";
import OfficeViewer from "./OfficeViewer";
import type { ViewerProps } from "./types";

/** A PDF's own pages, or its text in the shared document pane. The pages stay
 *  open behind the text, so going back to them does not fetch the file again. */
export default function PdfViewer(props: ViewerProps) {
  return (
    <>
      <div class="artifact-canvas-page" classList={{ resting: props.view === "text" }}>
        <PdfPages {...props} />
      </div>
      <Show when={props.view === "text"}>
        <OfficeViewer {...props} />
      </Show>
    </>
  );
}
