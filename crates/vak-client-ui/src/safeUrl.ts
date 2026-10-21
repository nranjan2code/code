// Single allow-list for "is this URL safe to turn into a live link/image src"
// on the desktop client, shared by both the live-streaming renderer (md.ts)
// and the settled/structured renderer (PresentationRenderer.tsx) so the two
// can never drift on what they consider navigable.
//
// This is deliberately a *subset* of the server's own allow-list
// (vak-delivery::presentation::safe_link/safe_media, crates/vak-delivery/src/presentation.rs)
// which additionally permits `#`-anchors and bare relative paths. Those are
// safe in a server-rendered/browser context but riskier inside a Tauri
// webview (a relative path can resolve against app-internal origins), so the
// client intentionally stays stricter — a link the server marks `safe` can
// still end up inert here. That's expected, not a bug: treat a widening of
// this list as a deliberate security decision, not routine parity work.
export function safeUrl(value: string, media = false): boolean {
  const normalized = value.trim().toLowerCase();
  // A scheme check is not permission to send model-visible data in a URL.
  // Active answer media must be self-contained; remote addresses remain links.
  if (media) return /^data:(image\/(png|jpe?g|gif|webp|avif)|audio\/[a-z0-9.+-]+|video\/[a-z0-9.+-]+)[;,]/.test(normalized);
  return (
    normalized.startsWith("https://") ||
    normalized.startsWith("http://") ||
    normalized.startsWith("mailto:")
  );
}

/**
 * Test whether a target URL/path represents a safe local workspace file or artifact reference.
 * Rejects dangerous or web schemes (javascript:, data:, vbscript:, http:, https:, //).
 */
export function isLocalArtifactPath(target: string): boolean {
  if (!target) return false;
  const raw = target.trim();
  const lower = raw.toLowerCase();
  if (
    lower.startsWith("javascript:") ||
    lower.startsWith("vbscript:") ||
    lower.startsWith("data:") ||
    lower.startsWith("http://") ||
    lower.startsWith("https://") ||
    lower.startsWith("mailto:") ||
    lower.startsWith("//")
  ) {
    return false;
  }
  const clean = lower.startsWith("file://") ? lower.slice(7) : lower;
  const stripped = clean.replace(/[.,;:!?)]'"`]+$/, "").trim();
  if (!stripped || stripped === "#") return false;
  return (
    stripped.startsWith("./") ||
    stripped.startsWith(".vak/") ||
    stripped.includes(".vak/scratch") ||
    /^[\w@.-]+(\/[\w@.-]+)*\.\w{1,6}$/.test(stripped) ||
    /^\.[\w/-]+$/.test(stripped)
  );
}

/**
 * Clean a local file/artifact reference to a relative workspace or scratch path.
 */
export function cleanArtifactPath(target: string): string {
  let clean = target.trim();
  if (clean.toLowerCase().startsWith("file://")) {
    clean = clean.slice(7);
  }
  if (clean.startsWith("./")) {
    clean = clean.slice(2);
  }
  return clean.replace(/[.,;:!?)]'"`]+$/, "").trim();
}

/**
 * The sandbox a preview frame runs in. The client owns this choice: preview
 * data (a card, a model, a page the model read) never supplies it, because
 * `allow-same-origin` on a `srcdoc` frame hands the previewed script the app's
 * own origin and, with it, the authenticated API.
 *
 * - `static` — markup from the conversation, and a page shown as one
 *   document: scripts are blocked in an opaque origin and forms can render, and
 *   nothing else.
 * - `interactive` — a human explicitly runs document code in an opaque
 *   frame; script downloads and fetch stay denied, but WebRTC may connect.
 * - `origin` — a page served from an origin of its own (a preview origin, or
 *   a dev server on its own port), framed under a different loopback name
 *   than the app's. It is not the app's origin, so it keeps its own storage,
 *   loads its own files, and may open windows.
 */
export type PreviewSandbox = "static" | "interactive" | "origin";

const PREVIEW_SANDBOX: Record<PreviewSandbox, string> = {
  static: "allow-forms",
  interactive: "allow-scripts allow-forms",
  origin: "allow-scripts allow-same-origin allow-forms allow-popups",
};

export function previewSandbox(kind: PreviewSandbox): string {
  return PREVIEW_SANDBOX[kind];
}

const PREVIEW_POLICY = [
  "default-src 'none'",
  "base-uri 'none'",
  "form-action 'none'",
  "script-src 'none'",
  "style-src 'unsafe-inline' data:",
  "img-src data: blob:",
  "font-src data: blob:",
  "media-src data: blob:",
  "connect-src 'none'",
].join("; ");

const PREVIEW_CSP_META = `<meta http-equiv="Content-Security-Policy" content="${PREVIEW_POLICY}">`;

/**
 * Constrain generated document previews. `sandbox` isolates the document
 * origin. The default blocks all page code and network fetches. The explicit
 * interactive mode permits embedded code and must disclose possible WebRTC
 * network access; it must never be described as offline.
 *
 * The policy is placed before the document's first byte of markup (after only
 * its doctype), so no script, wherever it sits relative to `<head>`, or
 * inside a string or comment that merely contains the text `<head>`, can run
 * before it applies. The HTML parser moves that leading `<meta>` into the head.
 */
export function sandboxedSrcdoc(html: string, mode: "static" | "interactive" = "static"): string {
  const doctype = /^\s*<!doctype[^>]*>/i.exec(html);
  const rest = doctype ? html.slice(doctype[0].length) : html;
  // Interactive mode is a human choice for this document only. Script URLs,
  // eval, fetch, frames and forms remain denied. WebRTC is outside fetch CSP:
  // callers must disclose possible network access, never describe it as offline.
  const policy = mode === "interactive"
    ? PREVIEW_CSP_META.replace("script-src 'none'", "script-src 'unsafe-inline'")
    : PREVIEW_CSP_META;
  return `${doctype ? doctype[0].trim() : "<!doctype html>"}${policy}${previewThemeStyle()}${rest}`;
}

/** The app's tokens a preview is given, by the name the page uses
 *  (docs/design/96-answer-projection-gaps.md Q1, docs/design/98 §5). */
const PREVIEW_TOKENS: ReadonlyArray<readonly [string, string]> = [
  ["--vak-bg", "--bg"],
  ["--vak-surface", "--surface"],
  ["--vak-fg", "--text"],
  ["--vak-soft", "--text-soft"],
  ["--vak-muted", "--muted"],
  ["--vak-accent", "--accent"],
  ["--vak-on-accent", "--on-accent"],
  ["--vak-rule", "--border"],
  ["--vak-good", "--green"],
  ["--vak-warn", "--yellow"],
  ["--vak-bad", "--red"],
  ["--vak-series-1", "--chart-1"],
  ["--vak-series-2", "--chart-2"],
  ["--vak-series-3", "--chart-3"],
  ["--vak-series-4", "--chart-4"],
  ["--vak-font", "--sans"],
  ["--vak-font-mono", "--mono"],
  ["--vak-radius", "--radius"],
];

/** How plain elements and the `vak-` classes look in a preview: the
 *  product's type scale and surfaces, at zero specificity (`:where`), so a
 *  page's own rules always win. */
const PREVIEW_BASE = [
  ":where(html){background:var(--vak-bg);color:var(--vak-fg);font-family:var(--vak-font);font-size:16px;line-height:1.55}",
  ":where(body){margin:0;padding:24px}",
  ":where(h1){font-size:1.75rem;line-height:1.2;margin:0 0 .5em}",
  ":where(h2){font-size:1.375rem;line-height:1.25;margin:1.2em 0 .5em}",
  ":where(h3){font-size:1.125rem;line-height:1.3;margin:1em 0 .4em}",
  ":where(a){color:var(--vak-accent)}",
  ":where(small,.vak-muted){color:var(--vak-muted);font-size:.875rem}",
  ":where(code,pre,kbd){font-family:var(--vak-font-mono);font-size:.875rem}",
  ":where(table){border-collapse:collapse;width:100%}",
  ":where(th,td){padding:8px 12px;border-bottom:1px solid var(--vak-rule);text-align:left}",
  ":where(th){color:var(--vak-muted);font-weight:600;font-size:.875rem}",
  ":where(button,input,select,textarea){font:inherit;font-size:.875rem;color:var(--vak-fg)}",
  ":where(button){min-height:32px;padding:0 12px;border:1px solid var(--vak-rule);border-radius:var(--vak-radius);background:var(--vak-surface);cursor:pointer}",
  ":where(button.vak-primary){background:var(--vak-accent);border-color:transparent;color:var(--vak-on-accent)}",
  ":where(input,select,textarea){min-height:32px;padding:0 8px;border:1px solid var(--vak-rule);border-radius:var(--vak-radius);background:var(--vak-surface)}",
  ":where(input[type=range],input[type=checkbox],input[type=radio],progress){accent-color:var(--vak-accent)}",
  ":where(:focus-visible){outline:2px solid var(--vak-accent);outline-offset:2px}",
  ":where(.vak-card){background:var(--vak-surface);border:1px solid var(--vak-rule);border-radius:var(--vak-radius);padding:16px}",
  ":where(.vak-grid){display:grid;gap:16px;grid-template-columns:repeat(auto-fit,minmax(min(100%,16rem),1fr))}",
].join("");

const THEME_STYLE = /<style data-vak-theme>[^<]*<\/style>/;

/**
 * The product's look as a stylesheet, read from the app's root style now:
 * its tokens as `--vak-*` variables and the base above. It holds no script
 * and no address. Markup previews carry it first; a page on a preview origin
 * is sent it (`postMessage`, `{type: "vak-theme", css}`).
 */
export function previewThemeCss(): string {
  if (typeof document === "undefined" || typeof getComputedStyle === "undefined") return "";
  const root = getComputedStyle(document.documentElement);
  const tokens = PREVIEW_TOKENS
    .map(([name, from]) => [name, root.getPropertyValue(from).trim().replace(/[<>;{}]/g, "")] as const)
    .filter(([, value]) => value)
    .map(([name, value]) => `${name}:${value}`)
    .join(";");
  const scheme = root.colorScheme === "dark" ? "dark" : "light";
  return `:root{${tokens};color-scheme:${scheme}}${PREVIEW_BASE}`;
}

function previewThemeStyle(): string {
  const css = previewThemeCss();
  return css ? `<style data-vak-theme>${css}</style>` : "";
}

/**
 * A built preview document with the theme as it is now: the leading theme
 * block is written again and nothing else changes, so a theme switch redraws
 * the open preview without reading its files again.
 */
export function withCurrentTheme(page: string): string {
  const style = previewThemeStyle();
  return style && THEME_STYLE.test(page) ? page.replace(THEME_STYLE, () => style) : page;
}

/**
 * The page a preview is opened in when it gets a window of its own. A `blob:`
 * page takes the origin of the app that made it, so the preview cannot be the
 * page: it is loaded into a sandboxed frame inside a bare wrapper that has no
 * script, and keeps the opaque origin and the no-network policy it has in the
 * Canvas.
 */
export function previewWindowDocument(page: string): string {
  const attribute = page.replace(/&/g, "&amp;").replace(/"/g, "&quot;");
  return `<!doctype html><meta charset="utf-8"><style>html,body,iframe{margin:0;width:100%;height:100%;border:0}</style><iframe sandbox="${PREVIEW_SANDBOX.static}" srcdoc="${attribute}"></iframe>`;
}

/** Pop out an origin-served document inside the same non-scripted wrapper.
 * Its parent frame policy still prevents refresh/navigation to an internet URL. */
export function previewOriginWindowDocument(url: string): string {
  const attribute = url.replace(/&/g, "&amp;").replace(/"/g, "&quot;");
  return `<!doctype html><meta charset="utf-8"><style>html,body,iframe{margin:0;width:100%;height:100%;border:0}</style><iframe sandbox="${PREVIEW_SANDBOX.static}" src="${attribute}"></iframe>`;
}
