import { subjectPath } from "../../canvasSubject";
import { subjectReader } from "../../artifactPreview";
import { createLoader, readText } from "./createLoader";
import LoadState from "./LoadState";
import SourcePane from "./SourcePane";
import type { ViewerProps } from "./types";

/** Any text file, shown as selectable source. */
export default function SourceViewer(props: ViewerProps) {
  const loader = createLoader(
    () => [props.subject, props.reloadKey] as const,
    ([subject]) => readText(subjectReader(subject), subjectPath(subject)),
  );
  return (
    <LoadState loader={loader}>{(text) =>
      <SourcePane text={text} label={subjectPath(props.subject)} selection={props.selection} onSelect={props.onSelect} />
    }</LoadState>
  );
}
