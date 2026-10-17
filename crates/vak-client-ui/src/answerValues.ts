import { createStore } from "solid-js/store";
import { evaluateExpression, parseExpression } from "./controlsExpression";

/** What each controls card publishes now, by its presentation id
 *  (docs/design/98 §2): its inputs as the person set them and its outputs. */
const [published, setPublishedStore] = createStore<Record<string, Record<string, number>>>({});

export function publish(presentationId: string, values: Record<string, number>) {
  setPublishedStore(presentationId, values);
}

type Binding = { path: (string | number)[]; expression: string; from: string[] };

function bindings(payload: Record<string, unknown>): Binding[] {
  return Array.isArray(payload.bound) ? (payload.bound as Binding[]) : [];
}

/** A card's payload with each bound field computed from what its source
 *  cards publish now; a field whose sources have published nothing yet
 *  keeps the value the host computed. */
export function withBoundValues(payload: Record<string, unknown>): Record<string, unknown> {
  const bound = bindings(payload);
  if (bound.length === 0) return payload;
  // A store proxy cannot be structured-cloned; the payload is plain JSON.
  const next = JSON.parse(JSON.stringify(payload)) as Record<string, unknown>;
  for (const binding of bound) {
    const values = Object.assign({}, ...binding.from.map((id) => published[id] ?? {}));
    if (binding.from.some((id) => !published[id])) continue;
    let value: number;
    try {
      value = evaluateExpression(parseExpression(binding.expression), values);
    } catch {
      continue;
    }
    let here: unknown = next;
    for (const step of binding.path.slice(0, -1)) here = (here as Record<string | number, unknown>)?.[step];
    const last = binding.path[binding.path.length - 1];
    if (here && typeof here === "object") (here as Record<string | number, unknown>)[last] = value;
  }
  return next;
}
