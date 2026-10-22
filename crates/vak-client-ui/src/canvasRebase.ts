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
  type EditField = "draft" | "checkpoints";
  const edits = new Map<string, Set<EditField>>();
  for (const op of pending) {
    if (op.op !== "update" || !localEntries.some(entry => entry.key === op.key)) continue;
    const fields = edits.get(op.key) ?? new Set<EditField>();
    for (const field of ["draft", "checkpoints"] as const) if (op.patch[field] !== undefined) fields.add(field);
    if (fields.size) edits.set(op.key, fields);
  }
  const draftKeys = new Set(edits.keys());
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
    const fields = [...edits.get(key)!];
    if (fields.some(field => {
      const base = JSON.stringify(old?.[field] ?? (field === "draft" ? "" : {}));
      const theirs = JSON.stringify(current[field] ?? (field === "draft" ? "" : {}));
      const ours = JSON.stringify(mine[field] ?? (field === "draft" ? "" : {}));
      return ours !== theirs && theirs !== base;
    })) {
      actualConflicts.add(key);
      if (!choices[key]) return { conflict: true };
    }
  }
  if (Object.keys(choices).some((key) => !actualConflicts.has(key))
    || [...actualConflicts].some((key) => !choices[key])) return { conflict: true };

  const replay = pending.flatMap((op): CanvasOp[] => {
    if (op.op !== "update" || choices[op.key] !== "saved" || (op.patch.draft === undefined && op.patch.checkpoints === undefined)) return [op];
    const { draft: _discardedDraft, checkpoints: _discardedCheckpoints, ...patch } = op.patch;
    return Object.keys(patch).length ? [{ ...op, patch }] : [];
  });
  for (const key of actualConflicts) if (choices[key] === "saved") draftKeys.delete(key);

  let stacks: CanvasStacks = withCanvas({}, "canvas", remote);
  for (const op of replay) {
    const finalMine = op.op === "update" ? localEntries.find((entry) => entry.key === op.key) : undefined;
    if (op.op === "update" && (op.patch.draft !== undefined || op.patch.checkpoints !== undefined) && finalMine && (!!finalMine.draft || Object.keys(finalMine.checkpoints ?? {}).length > 0) && !stacks.canvas?.entries.some((entry) => entry.key === op.key)) {
      const restored = openEntry(stacks, "canvas", finalMine.subject, fresh);
      if (restored === stacks) return { conflict: true };
      stacks = updateEntry(restored, "canvas", finalMine.key, {
        mode: finalMine.mode, view: finalMine.view, selection: finalMine.selection, draft: finalMine.draft, checkpoints: finalMine.checkpoints,
        feedbackOpen: finalMine.feedbackOpen, panel: finalMine.panel, dismissedVersion: finalMine.dismissedVersion,
      });
    }
    const before = stacks;
    const after = applyOp(stacks, "canvas", op, fresh);
    if (after === before) {
      const wouldLoseDraft = op.op === "open" || (op.op === "update" && (!!finalMine?.draft || Object.keys(finalMine?.checkpoints ?? {}).length > 0));
      if (wouldLoseDraft) return { conflict: true };
    }
    stacks = after;
  }

  // Operations can be older than the final local editor state. Preserve its
  // final note after replay, while retaining unrelated remote tabs and notes.
  for (const key of draftKeys) {
    const mine = localEntries.find((entry) => entry.key === key);
    if (!mine) return { conflict: true };
    if (!mine.draft && !Object.keys(mine.checkpoints ?? {}).length) continue;
    const present = stacks.canvas?.entries.find((entry) => entry.key === mine.key);
    if (!present) return { conflict: true };
    const patch: Partial<typeof mine> = {};
    // Preserve only the fields this surface edited. A workspace edit must
    // never carry its stale discussion note over a newer remote note (or vice versa).
    for (const field of edits.get(key)!) {
      if (JSON.stringify(present[field] ?? (field === "draft" ? "" : {}))
        !== JSON.stringify(mine[field] ?? (field === "draft" ? "" : {}))) Object.assign(patch, { [field]: mine[field] });
    }
    if (Object.keys(patch).length) stacks = updateEntry(stacks, "canvas", mine.key, patch);
  }
  return { canvas: stacks.canvas ?? null };
}
