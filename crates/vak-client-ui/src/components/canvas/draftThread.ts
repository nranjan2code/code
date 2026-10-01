import { createEffect, createMemo, createSignal, onCleanup } from "solid-js";
import * as api from "../../api";
import { watchCoworking } from "../../streamHub";
import type { CanvasSubject } from "../../canvasSubject";
import { newerVersions, versionNumber, versionsOf } from "../../draftVersions";

export type DraftSubject = Extract<CanvasSubject, { kind: "draft_file" }>;

/**
 * What surrounds a saved draft: the comments on this version, the versions
 * saved for its result and what happened to them, kept current while people
 * work on it together. Empty for anything that is not a saved draft. A newer
 * version is only ever reported here; it never replaces the one being read.
 */
export function createDraftThread(draft: () => DraftSubject | undefined) {
  const [comments, setComments] = createSignal<api.SandboxCandidateComment[]>([]);
  const [records, setRecords] = createSignal<api.SandboxRecord[]>([]);

  const versions = createMemo(() => versionsOf(records(), draft()?.executionId));
  const version = createMemo(() => {
    const subject = draft();
    return subject ? versionNumber(versions(), subject.candidateId) : null;
  });
  const newer = createMemo(() => {
    const subject = draft();
    return subject ? newerVersions(versions(), subject.candidateId) : [];
  });

  const load = async (subject: DraftSubject, alive: () => boolean) => {
    const [commentsResult, recordsResult] = await Promise.allSettled([
      api.listSandboxCandidateComments(subject.sessionId, subject.candidateId),
      api.listSessionSandboxRecords(subject.sessionId),
    ]);
    if (!alive()) return;
    // A failed refresh keeps what is already shown: offline is not an empty history.
    if (commentsResult.status === "fulfilled") setComments(commentsResult.value.comments);
    if (recordsResult.status === "fulfilled") setRecords(recordsResult.value.records);
  };

  createEffect(() => {
    const subject = draft();
    setComments([]);
    setRecords([]);
    if (!subject) return;
    let disposed = false;
    void load(subject, () => !disposed && draft()?.candidateId === subject.candidateId);
    const stop = watchCoworking(subject.sessionId, () => {
      void load(subject, () => !disposed && draft()?.candidateId === subject.candidateId);
    });
    onCleanup(() => { disposed = true; stop(); });
  });

  const refresh = (subject: DraftSubject) => load(subject, () => draft()?.candidateId === subject.candidateId);

  return { comments, records, versions, version, newer, refresh };
}

export type DraftThread = ReturnType<typeof createDraftThread>;
