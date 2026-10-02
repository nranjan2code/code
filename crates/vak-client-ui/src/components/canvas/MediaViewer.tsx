import { subjectKey, subjectPath } from "../../canvasSubject";
import { subjectReader } from "../../artifactPreview";
import { pdfAnchorPage } from "../../officeFiles";
import { createLoader } from "./createLoader";
import LoadState from "./LoadState";
import type { ViewerProps } from "./types";

/** Raw bytes of a file as a blob URL that is revoked when the viewer lets go
 *  of it. Read once for what the file is: moving to another cited place in it
 *  is not a new file. */
function createMedia(props: ViewerProps) {
  return createLoader(
    () => subjectKey(props.subject),
    async () => {
      const reader = subjectReader(props.subject);
      const file = subjectPath(props.subject);
      if (!reader || !file) throw new Error("There is no file to show here.");
      return reader.readFileRaw(file);
    },
    (url) => URL.revokeObjectURL(url),
  );
}

export function ImageViewer(props: ViewerProps) {
  const loader = createMedia(props);
  return (
    <LoadState loader={loader} label="Opening the picture…">{(url) =>
      <div class="artifact-canvas-image-container">
        <img class="artifact-canvas-image" src={url} alt={props.subject.title} />
      </div>
    }</LoadState>
  );
}

/** The browser's own reader over the file's pages, at the cited page when there is one. */
export function PdfPages(props: ViewerProps) {
  const loader = createMedia(props);
  return (
    <LoadState loader={loader} label="Opening the PDF…">{(url) => {
      const page = () => pdfAnchorPage(props.subject.anchor);
      return <iframe class="artifact-canvas-pdf-frame" src={page() ? `${url}#page=${page()}` : url} title={props.subject.title} />;
    }}</LoadState>
  );
}
