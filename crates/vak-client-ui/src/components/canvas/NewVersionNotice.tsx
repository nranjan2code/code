import { createMemo, Show } from "solid-js";
import { openArtifactCanvas, updateCanvasEntry } from "../../store";
import { fileSubject } from "../../canvasSubject";
import type { CanvasEntry } from "../../canvasStack";
import { changedFiles } from "../../draftVersions";
import type { DraftSubject, DraftThread } from "./draftThread";

/**
 * Says that a newer version of the draft being read has been saved, and offers
 * to read it. It never changes what is in front of the reader: the newer
 * version opens as a tab of its own, and comments stay on the version they
 * were written on.
 */
export default function NewVersionNotice(props: { entry: CanvasEntry; draft: DraftSubject; thread: DraftThread; onReview: (candidateId: string) => void }) {
  const latest = createMemo(() => props.thread.newer().at(-1));
  const latestNumber = () => props.thread.versions().length;
  const visible = () => !!latest() && latestNumber() > props.entry.dismissedVersion;
  const carriesFile = () => {
    const found = latest();
    return !!found && changedFiles(found).some((file) => file.path === props.draft.path && file.change !== "removed");
  };
  const read = () => {
    const found = latest();
    if (!found) return;
    openArtifactCanvas(fileSubject(props.draft.path, { sessionId: props.draft.sessionId, candidateId: found.candidate.candidate_id, executionId: props.draft.executionId, resultId: props.draft.resultId }, props.draft.title));
  };
  return (
    <Show when={visible()}>
      <div class="artifact-canvas-notice" role="status">
        <span>Version {latestNumber()} is ready. You are reading version {props.thread.version() ?? "—"}.</span>
        <Show when={carriesFile()}><button type="button" class="artifact-canvas-btn" onClick={read}>Read version {latestNumber()}</button></Show>
        <Show when={latest()}>{(found) => <button type="button" class="artifact-canvas-btn" onClick={() => props.onReview(found().candidate.candidate_id)}>Review changes</button>}</Show>
        <button type="button" class="artifact-canvas-btn" onClick={() => updateCanvasEntry({ dismissedVersion: latestNumber() }, props.entry.key)}>Dismiss</button>
      </div>
    </Show>
  );
}
