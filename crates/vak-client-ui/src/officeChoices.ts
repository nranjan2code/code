// Which of an Office draft's changes a person keeps (docs/design/72, P3).
// The server says which choices build on which (`requires`); this module
// only keeps a selection consistent with that, so a kept change never
// needs one that was left out. The server refuses such a selection anyway.

export type Requiring = { id: string; requires: string[] };

/** Leaves `id` out, and every choice that builds on it, directly or not. */
export function leaveOut(choices: Requiring[], excluded: ReadonlySet<string>, id: string): Set<string> {
  const next = new Set(excluded);
  const pending = [id];
  while (pending.length > 0) {
    const current = pending.pop()!;
    if (next.has(current)) continue;
    next.add(current);
    for (const choice of choices) {
      if (choice.requires.includes(current)) pending.push(choice.id);
    }
  }
  return next;
}

/** Keeps `id` again, and every choice it builds on, directly or not. */
export function keep(choices: Requiring[], excluded: ReadonlySet<string>, id: string): Set<string> {
  const next = new Set(excluded);
  const byId = new Map(choices.map((choice) => [choice.id, choice]));
  const pending = [id];
  while (pending.length > 0) {
    const current = pending.pop()!;
    if (!next.delete(current)) continue;
    pending.push(...(byId.get(current)?.requires ?? []));
  }
  return next;
}

/** The kept ids, in the draft's order. */
export function kept(choices: Requiring[], excluded: ReadonlySet<string>): string[] {
  return choices.filter((choice) => !excluded.has(choice.id)).map((choice) => choice.id);
}
