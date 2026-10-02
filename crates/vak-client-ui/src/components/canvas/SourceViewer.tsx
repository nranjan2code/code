import { subjectKey, subjectPath } from "../../canvasSubject";
import { subjectReader } from "../../artifactPreview";
import { createLoader, readText } from "./createLoader";
import LoadState from "./LoadState";
import SourcePane from "./SourcePane";
import type { ViewerProps } from "./types";

/** Any text file, shown as selectable source. */
export default function SourceViewer(props: ViewerProps) {
  const loader = createLoader(
    () => subjectKey(props.subject),
    () => readText(subjectReader(props.subject), subjectPath(props.subject)),
  );
  return (
    <LoadState loader={loader} label="Opening the file…">{(text) =>
      <SourcePane text={text} label={props.subject.title} path={subjectPath(props.subject)} selection={props.selection} onSelect={props.onSelect} />
    }</LoadState>
  );
}
