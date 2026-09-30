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
  if (media && normalized.startsWith("data:image/")) return true;
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
 * - `static` — generated documents and saved drafts: scripts run in an opaque
 *   origin and forms can render, and nothing else.
 * - `live_server` — a dev server on its own loopback port, so a different
 *   origin from the app already; it needs its own storage and popups.
 */
export type PreviewSandbox = "static" | "live_server";

const PREVIEW_SANDBOX: Record<PreviewSandbox, string> = {
  static: "allow-scripts allow-forms",
  live_server: "allow-scripts allow-same-origin allow-forms allow-popups",
};

export function previewSandbox(kind: PreviewSandbox): string {
  return PREVIEW_SANDBOX[kind];
}

const PREVIEW_POLICY = [
  "default-src 'none'",
  "base-uri 'none'",
  "form-action 'none'",
  "script-src 'unsafe-inline' blob:",
  "style-src 'unsafe-inline' data:",
  "img-src data: blob:",
  "font-src data: blob:",
  "media-src data: blob:",
  "connect-src 'none'",
].join("; ");

const PREVIEW_CSP_META = `<meta http-equiv="Content-Security-Policy" content="${PREVIEW_POLICY}">`;

/**
 * Constrain generated document previews. `sandbox` isolates the document
 * origin, while this CSP denies all network access, so what the chrome says
 * ("Safe preview, offline") is what the frame enforces.
 *
 * The policy is placed before the document's first byte of markup (after only
 * its doctype), so no script, wherever it sits relative to `<head>`, or
 * inside a string or comment that merely contains the text `<head>`, can run
 * before it applies. The HTML parser moves that leading `<meta>` into the head.
 */
export function sandboxedSrcdoc(html: string): string {
  const doctype = /^\s*<!doctype[^>]*>/i.exec(html);
  const rest = doctype ? html.slice(doctype[0].length) : html;
  return `${doctype ? doctype[0].trim() : "<!doctype html>"}${PREVIEW_CSP_META}${rest}`;
}
