import type { DiscoveredModels } from "./api";

/** A missing Bedrock authorization check is unknown, never permission. */
export function availableModels(result: DiscoveredModels): string[] {
  if (result.provider !== "bedrock") return result.models;
  if (result.availability_error) return [];
  return result.models.filter((model) => result.availability?.some((item) => item.model_id === model && item.invokable));
}

/** Retain a deliberate choice; a catalogue's ordering is not a recommendation. */
export function initialModel(models: readonly string[], current: string): string {
  return models.includes(current) ? current : models.length === 1 ? models[0] : "";
}
