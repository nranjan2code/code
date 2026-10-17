import { tokenThemeVariables } from "./components/presentation/MermaidViewer";

/**
 * The libraries a markup preview may name (docs/design/96 P3); the host's
 * `vak_delivery::skills::MARKUP_LIBRARIES` holds the same names. Each is the
 * client's own pinned build, read only when a preview names it and run
 * inside that preview's opaque frame, which has no network and no origin to
 * load anything from.
 */
const PREVIEW_LIBRARIES: Record<string, () => Promise<string>> = {
  diagram: async () => {
    const { default: source } = await import("mermaid/dist/mermaid.min.js?raw");
    const config = {
      startOnLoad: true,
      theme: "base",
      securityLevel: "strict",
      fontFamily: "var(--vak-font, sans-serif)",
      themeVariables: tokenThemeVariables(),
    };
    return `${source}\nmermaid.initialize(${JSON.stringify(config)});`;
  },
};

/** The markup with the named libraries' scripts placed at its end, so they
 *  run after the elements they draw. A name the client does not have is
 *  skipped: the host refuses one before a card is shown. */
export async function withPreviewLibraries(html: string, names: readonly unknown[]): Promise<string> {
  const scripts = await Promise.all(
    names
      .filter((name): name is string => typeof name === "string" && name in PREVIEW_LIBRARIES)
      .map((name) => PREVIEW_LIBRARIES[name]()),
  );
  if (scripts.length === 0) return html;
  const tags = scripts.map((script) => `<script>${script.replace(/<\/script/gi, "<\\/script")}</script>`).join("");
  return /<\/body>/i.test(html) ? html.replace(/<\/body>/i, `${tags}</body>`) : `${html}${tags}`;
}
