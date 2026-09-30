import { createEffect, createSignal, onCleanup } from "solid-js";
import * as api from "../../api";
import { watchCoworking } from "../../streamHub";
import type { CanvasSubject } from "../../canvasSubject";

export type DraftSubject = Extract<CanvasSubject, { kind: "draft_file" }>;

/**
 * The comments on a saved draft and which version of the draft it is, kept
 * current while people work on it together. Empty for anything that is not a
 * saved draft.
 */
export function createDraftThread(draft: () => DraftSubject | undefined) {
  const [comments, setComments] = createSignal<api.SandboxCandidateComment[]>([]);
  const [version, setVersion] = createSignal<number | null>(null);

  createEffect(() => {
    const subject = draft();
    setComments([]);
    setVersion(null);
    if (!subject) return;
    let disposed = false;
    void Promise.allSettled([
      api.listSandboxCandidateComments(subject.sessionId, subject.candidateId),
      api.listSessionSandboxRecords(subject.sessionId),
    ]).then(([commentsResult, recordsResult]) => {
      if (disposed) return;
      if (commentsResult.status === "fulfilled") setComments(commentsResult.value.comments);
      if (recordsResult.status === "fulfilled") {
        const versions = recordsResult.value.records.filter((record) => record.kind === "Candidate" && (!subject.executionId || record.record.execution_id === subject.executionId));
        const index = versions.findIndex((record) => record.kind === "Candidate" && record.record.candidate.candidate_id === subject.candidateId);
        if (index >= 0) setVersion(index + 1);
      }
    });
    onCleanup(() => { disposed = true; });
  });

  createEffect(() => {
    const subject = draft();
    if (!subject) return;
    const { sessionId, candidateId } = subject;
    let disposed = false;
    const stop = watchCoworking(sessionId, () => {
      void api.listSandboxCandidateComments(sessionId, candidateId)
        .then(({ comments: latest }) => {
          if (!disposed && draft()?.candidateId === candidateId) setComments(latest);
        })
        .catch(() => { /* Preserve the visible comment history while offline. */ });
    });
    onCleanup(() => { disposed = true; stop(); });
  });

  const refresh = (subject: DraftSubject) =>
    api.listSandboxCandidateComments(subject.sessionId, subject.candidateId)
      .then(({ comments: latest }) => setComments(latest))
      .catch(() => { /* The accepted comment remains durable. */ });

  return { comments, version, refresh };
}

export type DraftThread = ReturnType<typeof createDraftThread>;
