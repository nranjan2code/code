import { createSignal } from "solid-js";
import { withCurrentTheme } from "./safeUrl";

const [applied, setApplied] = createSignal("");

/** Called once the app's root carries a new theme, so every open preview
 *  takes it (docs/design/96-answer-projection-gaps.md, Q1). */
export function previewThemeChanged(key: string): void {
  setApplied(key);
}

/** A built preview document with the theme the app shows now; read inside a
 *  reactive scope, a theme switch rebuilds it. */
export function themedPreview(page: string): string {
  applied();
  return withCurrentTheme(page);
}
