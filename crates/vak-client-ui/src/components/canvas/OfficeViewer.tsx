import { activeId, coworkingPresence } from "../../store";
import { subjectCandidateId, subjectExecutionId, subjectPath, subjectSessionId } from "../../canvasSubject";
import OfficeWorkspacePane from "../OfficeWorkspacePane";
import type { ViewerProps } from "./types";

/** Word, Excel, PowerPoint and Visio files, and a PDF's text, drawn from the server's projection. */
export default function OfficeViewer(props: ViewerProps) {
  const sessionId = () => subjectSessionId(props.subject);
  const candidateId = () => subjectCandidateId(props.subject);
  return (
    <OfficeWorkspacePane
      hideHeader
      source={{ path: subjectPath(props.subject), sessionId: sessionId(), candidateId: candidateId(), executionId: subjectExecutionId(props.subject) }}
      fileName={props.subject.title}
      focus={props.subject.anchor}
      canEdit={true}
      canStart={!!sessionId() && !!candidateId()}
      collaborators={coworkingPresence(sessionId() ?? activeId())}
      onSelect={(anchor) => props.onSelect(anchor ? { kind: "anchor", anchor } : null)}
      onReview={subjectExecutionId(props.subject) ? props.onReview : undefined}
    />
  );
}
