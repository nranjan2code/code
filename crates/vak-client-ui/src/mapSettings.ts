import { createSignal } from "solid-js";
import * as api from "./api";
import type { MapSettings } from "./types";

/** `[maps]` as the server reports it (docs/design/96 P5): whether a map
 *  card may draw street tiles. Read once when a map is first drawn and
 *  replaced when the owner changes it in Settings. Unknown means off. */
const [settings, setSettings] = createSignal<MapSettings | undefined>();
let loading: Promise<void> | undefined;

export function mapSettings(): MapSettings | undefined {
  loading ??= api.getConfig().then((config) => { if (!settings()) setSettings(config.maps); }).catch(() => undefined);
  return settings();
}

export const setMapSettings = (next: MapSettings) => setSettings(next);
