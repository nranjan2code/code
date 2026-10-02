import { createEffect, createMemo, createSignal, on, onCleanup, untrack } from "solid-js";
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
 *
 * It follows which version is open, not the object naming it: the same
 * version named again (reopened, or the Canvas read back from the server)
 * keeps what is shown instead of clearing it and reading it again.
 */
export function createDraftThread(draft: () => DraftSubject | undefined) {
  const [comments, setComments] = createSignal<api.SandboxCandidateComment[]>([]);
  const [records, setRecords] = createSignal<api.SandboxRecord[]>([]);
  /** Nothing could be read yet: offline, or the server refused. */
  const [unavailable, setUnavailable] = createSignal(false);

  const identity = createMemo(() => {
    const subject = draft();
    return subject ? `${subject.sessionId}|${subject.candidateId}|${subject.executionId ?? ""}` : "";
  });
  const versions = createMemo(() => versionsOf(records(), draft()?.executionId));
  const version = createMemo(() => {
    const subject = draft();
    return subject ? versionNumber(versions(), subject.candidateId) : null;
  });
  const newer = createMemo(() => {
    const subject = draft();
    return subject ? newerVersions(versions(), subject.candidateId) : [];
  });

  let loaded = false;
  const load = async (subject: DraftSubject, alive: () => boolean) => {
    const [commentsResult, recordsResult] = await Promise.allSettled([
      api.listSandboxCandidateComments(subject.sessionId, subject.candidateId),
      api.listSessionSandboxRecords(subject.sessionId),
    ]);
    if (!alive()) return;
    // A failed refresh keeps what is already shown: offline is not an empty history.
    if (commentsResult.status === "fulfilled") setComments(commentsResult.value.comments);
    if (recordsResult.status === "fulfilled") setRecords(recordsResult.value.records);
    if (commentsResult.status === "fulfilled" || recordsResult.status === "fulfilled") loaded = true;
    setUnavailable(!loaded);
  };

  createEffect(on(identity, (key) => {
    setComments([]);
    setRecords([]);
    setUnavailable(false);
    loaded = false;
    const subject = untrack(draft);
    if (!key || !subject) return;
    let disposed = false;
    const alive = () => !disposed && identity() === key;
    void load(subject, alive);
    const stop = watchCoworking(subject.sessionId, () => void load(subject, alive));
    onCleanup(() => { disposed = true; stop(); });
  }));

  const refresh = (subject: DraftSubject) => load(subject, () => untrack(draft)?.candidateId === subject.candidateId);

  return { comments, records, versions, version, newer, unavailable, refresh };
}

export type DraftThread = ReturnType<typeof createDraftThread>;
