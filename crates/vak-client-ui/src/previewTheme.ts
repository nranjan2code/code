import { createSignal } from "solid-js";
import { previewThemeCss, withCurrentTheme } from "./safeUrl";

const [applied, setApplied] = createSignal("");

/** Called once the app's root carries a new theme, so every open preview
 *  takes it (docs/design/96-answer-projection-gaps.md, Q1). */
export function previewThemeChanged(key: string): void {
  setApplied(key);
}

/** Send the product's stylesheet to a page on a preview origin, which the
 *  preview server gave a slot for it (docs/design/98 §5). Read inside a
 *  reactive scope, a theme switch sends it again. */
export function sendPreviewTheme(frame: HTMLIFrameElement | undefined, url: string): void {
  applied();
  if (!frame?.contentWindow) return;
  let origin: string;
  try {
    origin = new URL(url).origin;
  } catch {
    return;
  }
  frame.contentWindow.postMessage({ type: "vak-theme", css: previewThemeCss() }, origin);
}

/** A built preview document with the theme the app shows now; read inside a
 *  reactive scope, a theme switch rebuilds it. */
export function themedPreview(page: string): string {
  applied();
  return withCurrentTheme(page);
}
