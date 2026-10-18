import { applyOp, fromStored, openEntry, updateEntry, withCanvas, type CanvasOp, type CanvasStacks, type ConversationCanvas, type FreshEntry, type StoredCanvasShape } from "./canvasStack.ts";

/** Rebuild a stale local Canvas over the latest server snapshot without
 * silently dropping a draft or replacing a concurrently edited note. */
export function rebaseCanvas(
  local: ConversationCanvas | undefined,
  stored: StoredCanvasShape,
  baseline: StoredCanvasShape | undefined,
  pending: readonly CanvasOp[],
  fresh: FreshEntry,
  choices: Readonly<Record<string, "mine" | "saved">> = {},
): { canvas: ConversationCanvas | null } | { conflict: true } {
  const remote = fromStored(stored, local, fresh);
  const localEntries = local?.entries ?? [];
  const remoteEntries = remote?.entries ?? [];
  const draftKeys = new Set(pending.flatMap((op) => op.op === "update" && op.patch.draft !== undefined
    && localEntries.some((entry) => entry.key === op.key) ? [op.key] : []));
  const actualConflicts = new Set<string>();

  // Only a queued draft edit protects local text. A read with no draft edit
  // accepts the latest server value, even when this tab has an old note.
  for (const key of draftKeys) {
    const mine = localEntries.find((entry) => entry.key === key);
    if (!mine) return { conflict: true };
    const current = remoteEntries.find((entry) => entry.key === key);
    // A remotely closed tab is not a competing edit. The queued local draft
    // is restored below, subject to the existing capacity check.
    if (!current) continue;
    const old = baseline?.entries.find((entry) => entry.key === key);
    const baselineDraft = typeof old?.draft === "string" ? old.draft : "";
    if (mine.draft !== (current.draft ?? "") && current.draft !== baselineDraft) {
      actualConflicts.add(key);
      if (!choices[key]) return { conflict: true };
    }
  }
  if (Object.keys(choices).some((key) => !actualConflicts.has(key))
    || [...actualConflicts].some((key) => !choices[key])) return { conflict: true };

  const replay = pending.flatMap((op): CanvasOp[] => {
    if (op.op !== "update" || choices[op.key] !== "saved" || op.patch.draft === undefined) return [op];
    const { draft: _discardedDraft, ...patch } = op.patch;
    return Object.keys(patch).length ? [{ ...op, patch }] : [];
  });
  for (const key of actualConflicts) if (choices[key] === "saved") draftKeys.delete(key);

  let stacks: CanvasStacks = withCanvas({}, "canvas", remote);
  for (const op of replay) {
    const finalMine = op.op === "update" ? localEntries.find((entry) => entry.key === op.key) : undefined;
    if (op.op === "update" && op.patch.draft !== undefined && finalMine?.draft && !stacks.canvas?.entries.some((entry) => entry.key === op.key)) {
      const restored = openEntry(stacks, "canvas", finalMine.subject, fresh);
      if (restored === stacks) return { conflict: true };
      stacks = updateEntry(restored, "canvas", finalMine.key, {
        mode: finalMine.mode, view: finalMine.view, selection: finalMine.selection, draft: finalMine.draft,
        feedbackOpen: finalMine.feedbackOpen, panel: finalMine.panel, dismissedVersion: finalMine.dismissedVersion,
      });
    }
    const before = stacks;
    const after = applyOp(stacks, "canvas", op, fresh);
    if (after === before) {
      const wouldLoseDraft = op.op === "open" || (op.op === "update" && typeof op.patch.draft === "string" && !!finalMine?.draft);
      if (wouldLoseDraft) return { conflict: true };
    }
    stacks = after;
  }

  // Operations can be older than the final local editor state. Preserve its
  // final note after replay, while retaining unrelated remote tabs and notes.
  for (const key of draftKeys) {
    const mine = localEntries.find((entry) => entry.key === key);
    if (!mine) return { conflict: true };
    if (!mine.draft) continue;
    const present = stacks.canvas?.entries.find((entry) => entry.key === mine.key);
    if (!present) return { conflict: true };
    if (present.draft !== mine.draft) {
      const remoteEntry = remoteEntries.find((entry) => entry.key === mine.key);
      const old = baseline?.entries.find((entry) => entry.key === mine.key);
      const remoteChanged = remoteEntry && remoteEntry.draft !== (typeof old?.draft === "string" ? old.draft : "");
      if (choices[key] !== "mine" && remoteChanged && remoteEntry.draft !== mine.draft) return { conflict: true };
      stacks = updateEntry(stacks, "canvas", mine.key, { draft: mine.draft });
    }
  }
  return { canvas: stacks.canvas ?? null };
}
