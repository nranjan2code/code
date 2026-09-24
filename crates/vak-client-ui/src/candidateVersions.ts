import type { SandboxCandidateRecord, SandboxPromotionRecord, SandboxRecord } from "./api";

/** The saved versions of an execution's result still waiting for review.
 *  Versions of one result are alternatives (a revision, or a version that
 *  keeps some of a draft's changes), so accepting one settles every version
 *  saved before it. A version saved after that acceptance starts a new
 *  round; undoing the acceptance reopens the round it closed. Records are
 *  in the order they were appended. */
export function pendingVersions(records: SandboxRecord[], executionId: string): SandboxCandidateRecord[] {
  const undone = undoneCandidates(records);
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

/** The acceptance of this execution's result that Undo would reverse: its
 *  latest Promotion not yet undone. Undoing it offers the one before, the
 *  same way undoing reopens a round in `pendingVersions`; the server still
 *  refuses an undo that a later workspace change would overwrite. Derived
 *  from the durable records alone, so a reload or another device sees the
 *  same offer as the person who clicked Accept. */
export function undoablePromotion(records: SandboxRecord[], executionId: string): SandboxPromotionRecord | null {
  const undone = undoneCandidates(records);
  const ours = new Set(records.flatMap((record) => record.kind === "Candidate" && record.record.execution_id === executionId ? [record.record.candidate.candidate_id] : []));
  return records.findLast((record): record is Extract<SandboxRecord, { kind: "Promotion" }> => record.kind === "Promotion" && ours.has(record.record.candidate_id) && !undone.has(record.record.candidate_id))?.record ?? null;
}

/** What an acceptance did, in the words shown beside its Undo. */
export function acceptanceSummary(promotion: SandboxPromotionRecord): string {
  const applied = `Applied ${promotion.receipt.verification?.length ?? 0} change(s).`;
  const integration = promotion.receipt.integration;
  return integration?.workspace_state_status === "observed" ? `${applied} Exact workspace state verified; target checks ${integration.target_checks_status}.` : applied;
}

function undoneCandidates(records: SandboxRecord[]): Set<string> {
  return new Set(records.flatMap((record) => record.kind === "PromotionUndo" ? [record.record.candidate_id] : []));
}
