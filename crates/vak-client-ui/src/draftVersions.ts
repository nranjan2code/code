// The saved versions of a draft, from the durable sandbox records
// (docs/plans/canvas-plan.md K4). One result can have several versions, and a
// comment belongs to the version it was written on: a newer version never takes
// over the one being read, and old comments never move to new line numbers.

import type { SandboxCandidateRecord, SandboxRecord } from "./api";

/** Every version saved for an execution's result, oldest first. */
export function versionsOf(records: readonly SandboxRecord[], executionId: string | undefined): SandboxCandidateRecord[] {
  if (!executionId) return [];
  return records.flatMap((record) => (record.kind === "Candidate" && record.record.execution_id === executionId ? [record.record] : []));
}

/** The 1-based number of a version among its result's versions, or null when it is not among them. */
export function versionNumber(versions: readonly SandboxCandidateRecord[], candidateId: string): number | null {
  const index = versions.findIndex((version) => version.candidate.candidate_id === candidateId);
  return index >= 0 ? index + 1 : null;
}

/** Versions saved after the one being read. */
export function newerVersions(versions: readonly SandboxCandidateRecord[], candidateId: string): SandboxCandidateRecord[] {
  const index = versions.findIndex((version) => version.candidate.candidate_id === candidateId);
  return index >= 0 ? versions.slice(index + 1) : [];
}

export type ChangedFile = { path: string; change: "new" | "changed" | "removed" };

/** What a version does to each file it names. A file with no base was not there before. */
export function changedFiles(version: SandboxCandidateRecord): ChangedFile[] {
  return version.candidate.files.map((file) => ({
    path: file.path,
    change: file.operation === "Delete" ? "removed" : file.base_hash ? "changed" : "new",
  }));
}

export type ActivityItem = { when: string; text: string };

/** What happened to a draft, newest first, from records and comments that exist; nothing is inferred. */
export function draftActivity(
  records: readonly SandboxRecord[],
  versions: readonly SandboxCandidateRecord[],
  comments: ReadonlyArray<{ actor_id: string; actor_name?: string; created_at?: string; text: string }>,
): ActivityItem[] {
  const items: ActivityItem[] = [];
  versions.forEach((version, index) => items.push({ when: version.updated_at, text: `Version ${index + 1} saved` }));
  const ids = new Set(versions.map((version) => version.candidate.candidate_id));
  for (const record of records) {
    if (record.kind !== "CandidateRevision" || !ids.has(record.record.parent_candidate_id)) continue;
    const number = versionNumber(versions, record.record.parent_candidate_id);
    const status = record.record.status === "Running" ? "is revising" : record.record.status === "Completed" ? "finished revising" : "could not revise";
    items.push({ when: record.record.updated_at, text: `The Agent ${status} version ${number}` });
  }
  for (const comment of comments) {
    if (!comment.created_at) continue;
    items.push({ when: comment.created_at, text: `${comment.actor_id === "operator" ? "You" : comment.actor_name ?? comment.actor_id} commented` });
  }
  return items.sort((a, b) => Date.parse(b.when) - Date.parse(a.when));
}
