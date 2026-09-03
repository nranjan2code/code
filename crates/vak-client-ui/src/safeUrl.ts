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
