import { createContext } from "solid-js";
import { createStore, reconcile as reconcileStore } from "solid-js/store";
import { evaluateExpression, parseExpression } from "./controlsExpression.ts";

/** Values and the person's current input text, shared by views of one answer. */
export type ControlSettings = Record<string, string>;
export type Published = Record<string, number | [number, number][]>;

export type AnswerValuesScope = {
  publish(sourceId: string, values: Published): void;
  clearPublication(sourceId: string): void;
  getSettings(sourceId: string): ControlSettings | undefined;
  setSettings(sourceId: string, values: ControlSettings): void;
  withBoundValues(payload: Record<string, unknown>): Record<string, unknown>;
};

export const AnswerValuesContext = createContext<AnswerValuesScope>();

type Entry = {
  scope: AnswerValuesScope;
  publications: Record<string, Published>;
  settings: Record<string, ControlSettings>;
  setPublications: (id: string, values: Published | undefined) => void;
  setSettingsStore: (id: string, values: ControlSettings | undefined) => void;
  mounts: Map<object, Set<string>>;
  sourceOwners: Map<string, number>;
};
const scopes = new Map<string, Entry>();

/** Acquire one mount of a keyed scope. Call reconcile as its rendered source
 * set changes, then release from the owning wrapper's cleanup. */
export function acquireAnswerValuesScope(key: string) {
  let entry = scopes.get(key);
  if (!entry) {
    const [publications, setPublications] = createStore<Record<string, Published>>({});
    const [settings, setSettingsStore] = createStore<Record<string, ControlSettings>>({});
    const sourceOwners = new Map<string, number>();
    const scope: AnswerValuesScope = {
      publish: (id, values) => { if (sourceOwners.has(id)) setPublications(id, reconcileStore(values)); },
      clearPublication: (id) => { if (sourceOwners.has(id)) setPublications(id, undefined as never); },
      getSettings: (id) => settings[id],
      setSettings: (id, values) => { if (sourceOwners.has(id)) setSettingsStore(id, reconcileStore(values)); },
      withBoundValues: (payload) => withBoundValuesFrom(publications, payload),
    };
    entry = {
      scope, publications, settings,
      setPublications: (id, values) => setPublications(id, values as Published),
      setSettingsStore: (id, values) => setSettingsStore(id, values as ControlSettings),
      mounts: new Map(), sourceOwners,
    };
    scopes.set(key, entry);
  }
  const ownedEntry = entry!;
  const token = {};
  ownedEntry.mounts.set(token, new Set());
  const reconcile = (ids: Iterable<string>) => {
    const next = new Set(ids);
    const current = ownedEntry.mounts.get(token);
    if (!current) return;
    for (const id of current) {
      if (next.has(id)) continue;
      const remaining = (ownedEntry.sourceOwners.get(id) ?? 1) - 1;
      if (remaining <= 0) {
        ownedEntry.sourceOwners.delete(id);
        ownedEntry.setPublications(id, undefined);
        ownedEntry.setSettingsStore(id, undefined);
      } else ownedEntry.sourceOwners.set(id, remaining);
    }
    for (const id of next) {
      if (current.has(id)) continue;
      ownedEntry.sourceOwners.set(id, (ownedEntry.sourceOwners.get(id) ?? 0) + 1);
    }
    ownedEntry.mounts.set(token, next);
  };
  let released = false;
  return {
    scope: ownedEntry.scope,
    reconcile,
    release() {
      if (released) return;
      released = true;
      reconcile([]);
      ownedEntry.mounts.delete(token);
      if (ownedEntry.mounts.size === 0) {
        for (const id of Object.keys(ownedEntry.publications)) ownedEntry.setPublications(id, undefined);
        for (const id of Object.keys(ownedEntry.settings)) ownedEntry.setSettingsStore(id, undefined);
        scopes.delete(key);
      }
    },
  };
}

type Binding = { path: (string | number)[]; expression?: string; series?: string; from: string[] };
function bindings(payload: Record<string, unknown>): Binding[] {
  return Array.isArray(payload.bound) ? payload.bound as Binding[] : [];
}

function withBoundValuesFrom(publications: Record<string, Published>, payload: Record<string, unknown>): Record<string, unknown> {
  const bound = bindings(payload);
  if (bound.length === 0) return payload;
  const next = JSON.parse(JSON.stringify(payload)) as Record<string, unknown>;
  for (const binding of bound) {
    if (binding.from.some((id) => !publications[id])) continue;
    const merged: Published = Object.assign({}, ...binding.from.map((id) => publications[id]));
    let value: unknown;
    if (binding.series) {
      const points = merged[binding.series];
      if (!Array.isArray(points)) continue;
      value = points.map(([x, y]) => ({ x, y }));
    } else {
      const numbers = Object.fromEntries(Object.entries(merged).filter((entry): entry is [string, number] => typeof entry[1] === "number"));
      try { value = evaluateExpression(parseExpression(binding.expression ?? ""), numbers); }
      catch { continue; }
    }
    let here: unknown = next;
    for (const step of binding.path.slice(0, -1)) here = (here as Record<string | number, unknown>)?.[step];
    const last = binding.path[binding.path.length - 1];
    if (here && typeof here === "object" && last !== undefined) (here as Record<string | number, unknown>)[last] = value;
  }
  return next;
}
