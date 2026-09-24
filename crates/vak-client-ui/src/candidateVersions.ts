import type { SandboxCandidateRecord, SandboxRecord } from "./api";

/** The saved versions of an execution's result still waiting for review.
 *  Versions of one result are alternatives (a revision, or a version that
 *  keeps some of a draft's changes), so accepting one settles every version
 *  saved before it. A version saved after that acceptance starts a new
 *  round; undoing the acceptance reopens the round it closed. Records are
 *  in the order they were appended. */
export function pendingVersions(records: SandboxRecord[], executionId: string): SandboxCandidateRecord[] {
  const undone = new Set(records.filter((record) => record.kind === "PromotionUndo").map((record) => record.record.candidate_id));
  const promoted = new Set<string>();
  const ours = new Set<string>();
  let round: SandboxCandidateRecord[] = [];
  for (const record of records) {
    if (record.kind === "Candidate" && record.record.execution_id === executionId) {
      ours.add(record.record.candidate.candidate_id);
      round.push(record.record);
    } else if (record.kind === "Promotion") {
      promoted.add(record.record.candidate_id);
      if (ours.has(record.record.candidate_id) && !undone.has(record.record.candidate_id)) round = [];
    }
  }
  return round.filter((record) => !promoted.has(record.candidate.candidate_id));
}
