import { itemsOf } from "./store";

/** One part of the running turn's plan, and whether a card delivered it. */
export type PlanPart = { label: string; delivered: boolean };

/** The parts the running turn's plan named, each marked delivered once a
 *  card naming it as its `deliverable` was accepted; `undefined` when the
 *  turn made no plan (docs/design/96, live turn). */
export function livePlan(sessionId: string | null): PlanPart[] | undefined {
  if (!sessionId) return undefined;
  const items = itemsOf(sessionId);
  const turn = items.slice(items.map((item) => item.kind).lastIndexOf("user") + 1);
  const plan = [...turn].reverse().find((item) => item.kind === "tool" && item.name === "plan");
  if (plan?.kind !== "tool") return undefined;
  let names: string[] = [];
  try {
    const parsed = JSON.parse(plan.argsJson) as { deliverables?: { name?: string }[] };
    names = (parsed.deliverables ?? []).map((part) => part.name?.trim() ?? "").filter(Boolean);
  } catch {
    return undefined;
  }
  const delivered = new Set(turn.flatMap((item) => {
    if (item.kind !== "tool" || !item.done || item.isError || item.name === "plan") return [];
    try {
      const name = (JSON.parse(item.argsJson) as { deliverable?: string }).deliverable;
      return name ? [name.trim().toLowerCase()] : [];
    } catch {
      return [];
    }
  }));
  return names.map((name) => ({ label: name.replace(/_/g, " "), delivered: delivered.has(name.toLowerCase()) }));
}
