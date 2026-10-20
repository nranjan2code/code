/** External navigation is a person's action, never a renderer side effect. */
export function externalLink(value: string): string | null {
  if (value.length > 8192 || /[\u0000-\u001f\u007f]/.test(value)) return null;
  try {
    const url = new URL(value);
    return ["https:", "http:", "mailto:"].includes(url.protocol) && !url.username && !url.password
      ? url.href : null;
  } catch {
    return null;
  }
}

/** Delegation also covers links inserted by streamed Markdown. Relative app
 * routes and download links keep their existing handling. */
export function installExternalLinks(root: Document, openNative?: (url: string) => Promise<void>): () => void {
  const activate = (event: MouseEvent) => {
    if (event.defaultPrevented || (event.type === "auxclick" ? event.button !== 1 : event.button !== 0)) return;
    const anchor = event.composedPath().find((node): node is HTMLAnchorElement => node instanceof HTMLAnchorElement);
    if (!anchor || anchor.hasAttribute("download")) return;
    const value = anchor.getAttribute("href") ?? "";
    const url = externalLink(value);
    if (!url) {
      // A rejected absolute URL must not fall through to browser navigation.
      // Relative application/artifact links keep their own click handlers.
      if (/^[a-z][a-z0-9+.-]*:|^\/\//i.test(value.trim())) event.preventDefault();
      return;
    }
    if (!event.isTrusted) { event.preventDefault(); return; }
    // Explicit same-origin links to the app are still application navigation.
    if (new URL(url).origin === root.location.origin) return;
    anchor.rel = "noopener noreferrer";
    anchor.referrerPolicy = "no-referrer";
    anchor.target = "_blank";
    if (openNative) {
      event.preventDefault();
      void openNative(url).catch(() => {
        // Keep a visible failure without exposing URL contents in logs.
        root.defaultView?.alert("Could not open this link in your browser.");
      });
    }
  };
  root.addEventListener("click", activate, true);
  root.addEventListener("auxclick", activate, true);
  return () => {
    root.removeEventListener("click", activate, true);
    root.removeEventListener("auxclick", activate, true);
  };
}
