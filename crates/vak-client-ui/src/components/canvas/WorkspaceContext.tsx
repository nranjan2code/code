import { createContext, onCleanup, useContext, type ParentProps } from "solid-js";
import type { CanvasViewport } from "../../canvasDrawing";
import type { WorkspaceCheckpoint, WorkspaceValue } from "../../canvasWorkspace";

/** Services handed to owned workspace renderers by the common web/desktop host. */
export interface CanvasWorkspace {
  /** Aborted on tab unmount or renderer replacement; late edits are ignored. */
  readonly signal: AbortSignal;
  viewport(): CanvasViewport;
  read(name: string): WorkspaceCheckpoint | undefined;
  checkpoint(name: string, value: WorkspaceCheckpoint): void;
  clear(name: string): void;
}

export const CanvasWorkspaceContext = createContext<CanvasWorkspace>();
export const useCanvasWorkspace = () => useContext(CanvasWorkspaceContext);

export function checkpointValue(value: unknown): WorkspaceValue {
  // Checkpoints contain values, never reactive proxies or runtime handles.
  return JSON.parse(JSON.stringify(value)) as WorkspaceValue;
}

/** A contribution replacement revokes its callbacks even while the tab stays open. */
export function CanvasWorkspaceLease(props: ParentProps) {
  const parent = useCanvasWorkspace();
  if (!parent) return props.children;
  const controller = new AbortController();
  const abort = () => controller.abort();
  if (parent.signal.aborted) abort();
  else parent.signal.addEventListener("abort", abort, { once: true });
  onCleanup(() => { abort(); parent.signal.removeEventListener("abort", abort); });
  const live = () => !controller.signal.aborted;
  const value: CanvasWorkspace = {
    viewport: parent.viewport, signal: controller.signal,
    read: name => live() ? parent.read(name) : undefined,
    checkpoint: (name, checkpoint) => { if (live()) parent.checkpoint(name, checkpoint); },
    clear: name => { if (live()) parent.clear(name); },
  };
  return <CanvasWorkspaceContext.Provider value={value}>{props.children}</CanvasWorkspaceContext.Provider>;
}
