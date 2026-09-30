import type { DiscoveredModels } from "./api";

/** Keep the live provider catalogue as the source of model choices. */
export function availableModels(result: DiscoveredModels): string[] {
  return result.models;
}

/** Retain a deliberate choice; a catalogue's ordering is not a recommendation. */
export function initialModel(models: readonly string[], current: string): string {
  return models.includes(current) ? current : models.length === 1 ? models[0] : "";
}
