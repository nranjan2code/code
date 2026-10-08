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
  const [comments, setComments] = createSignal<api.VersionComment[]>([]);
  /** The artifact version this draft proposes: where its thread is written. */
  const [binding, setBinding] = createSignal<api.VersionBinding | null>(null);
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
    // A failed refresh keeps what is already shown: offline is not an empty history.
    try {
      const listed = await api.listSessionSandboxRecords(subject.sessionId);
      if (!alive()) return;
      setRecords(listed.records);
      loaded = true;
      const found = api.knownVersion(subject.sessionId, subject.candidateId, subject.path);
      setBinding(found);
      if (found) {
        const thread = await api.versionComments(found.artifact, found.version);
        if (!alive()) return;
        setComments(thread.comments);
      }
    } catch { /* keep what is shown */ }
    if (!alive()) return;
    setUnavailable(!loaded);
  };

  createEffect(on(identity, (key) => {
    setComments([]);
    setBinding(null);
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

  return { comments, binding, records, versions, version, newer, unavailable, refresh };
}

export type DraftThread = ReturnType<typeof createDraftThread>;
