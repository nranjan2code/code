import { createSignal, type Component } from "solid-js";
import type { ViewerProps } from "./components/canvas/types";
import type { ViewerSpec } from "./canvasViewers";

export interface CanvasWorkspaceDefinition {
  /** Namespaced identity of an owned, compiled renderer. Packages never inject host code. */
  id: string;
  spec: Omit<ViewerSpec, "type">;
  component: Component<ViewerProps>;
}
const definitions = new Map<string, CanvasWorkspaceDefinition>();
const [generation, setGeneration] = createSignal(0);

/** Registration changes presentation availability; it never grants data or execution authority. */
export function registerCanvasWorkspace(definition: CanvasWorkspaceDefinition): () => void {
  if (!/^[a-z][a-z0-9_.-]*\/[a-z][a-z0-9_.-]*$/.test(definition.id)) throw new Error("A workspace needs a namespaced identity.");
  if (definitions.has(definition.id)) throw new Error("This workspace is already registered.");
  definitions.set(definition.id, definition);
  setGeneration(value => value + 1);
  return () => {
    if (definitions.get(definition.id) !== definition) return;
    definitions.delete(definition.id);
    setGeneration(value => value + 1);
  };
}

export function canvasWorkspace(id: string): CanvasWorkspaceDefinition | undefined {
  generation();
  return definitions.get(id);
}
