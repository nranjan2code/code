/** Recoverable edits belong to the work, independently of a mounted renderer. */
export type WorkspaceValue = null | boolean | number | string | WorkspaceValue[] | { [key: string]: WorkspaceValue };
export interface WorkspaceCheckpoint {
  schema: number;
  summary: string;
  data: WorkspaceValue;
}
export type WorkspaceCheckpoints = Readonly<Record<string, WorkspaceCheckpoint>>;

/** Checkpoints are bounded JSON values, never cyclic graphs, accessors or handles. */
export function isWorkspaceValue(value: unknown): value is WorkspaceValue {
  const ancestors = new Set<object>();
  let remaining = 100_000;
  const valid = (item: unknown, depth: number): boolean => {
    if (--remaining < 0 || depth > 64) return false;
    if (item === null || typeof item === "string" || typeof item === "boolean") return true;
    if (typeof item === "number") return Number.isFinite(item);
    if (!item || typeof item !== "object" || ancestors.has(item)) return false;
    if (!Array.isArray(item) && Object.getPrototypeOf(item) !== Object.prototype) return false;
    if (Object.getOwnPropertySymbols(item).length) return false;
    const descriptors = Object.getOwnPropertyDescriptors(item);
    if (Object.values(descriptors).some(field => field.get || field.set)) return false;
    ancestors.add(item);
    const result = Array.isArray(item)
      ? Object.keys(item).length === item.length && item.every(value => valid(value, depth + 1))
      : Object.values(descriptors).every(field => field.enumerable && valid(field.value, depth + 1));
    ancestors.delete(item);
    return result;
  };
  return valid(value, 0);
}

export function validCheckpoints(value: unknown): value is WorkspaceCheckpoints {
  if (!isWorkspaceValue(value) || !value || typeof value !== "object" || Array.isArray(value)) return false;
  return Object.entries(value).every(([key, item]) => {
    if (!key || !item || typeof item !== "object" || Array.isArray(item)) return false;
    return Object.keys(item).every(field => ["schema", "summary", "data"].includes(field))
      && Number.isSafeInteger(item.schema) && (item.schema as number) > 0
      && typeof item.summary === "string" && Object.hasOwn(item, "data");
  });
}

/** Detach extension-owned input before it enters shared/recoverable state. */
export function copyCheckpoint(value: WorkspaceCheckpoint): WorkspaceCheckpoint {
  if (!validCheckpoints({ checkpoint: value })) throw new Error("The workspace checkpoint is invalid.");
  return JSON.parse(JSON.stringify(value)) as WorkspaceCheckpoint;
}

/** A mounted renderer has a revocable editing lease, not a reference to the store. */
export function createWorkspaceEdits(
  read: () => WorkspaceCheckpoints | undefined,
  write: (checkpoints: WorkspaceCheckpoints) => void,
) {
  const controller = new AbortController();
  return {
    signal: controller.signal,
    read(name: string): WorkspaceCheckpoint | undefined {
      if (controller.signal.aborted) return undefined;
      const current = read();
      return current && Object.hasOwn(current, name) ? copyCheckpoint(current[name]) : undefined;
    },
    checkpoint(name: string, value: WorkspaceCheckpoint) {
      if (controller.signal.aborted) return;
      const checkpoints = { ...read(), [name]: copyCheckpoint(value) };
      if (!validCheckpoints(checkpoints)) throw new Error("The workspace checkpoint is invalid.");
      if (JSON.stringify(read() ?? {}) !== JSON.stringify(checkpoints)) write(checkpoints);
    },
    clear(name: string) {
      if (controller.signal.aborted || !Object.hasOwn(read() ?? {}, name)) return;
      const checkpoints = { ...read() };
      delete checkpoints[name];
      write(checkpoints);
    },
    dispose() { controller.abort(); },
  };
}
