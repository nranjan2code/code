//! Syntax highlighting, shared by the editor pane and chat code blocks.
//!
//! Shiki is loaded lazily and only for the languages actually asked for, so
//! opening a Python file never pays for the Rust or TypeScript grammars. The
//! highlighter is a singleton: creating one per call is what makes naive
//! Shiki integrations slow.

import type { HighlighterCore } from "shiki";

let corePromise: Promise<HighlighterCore> | null = null;
const loaded = new Set<string>();

/**
 * Extension → Shiki language id. Unknown extensions fall back to plain text
 * rather than guessing: a wrong grammar colors code misleadingly, which is
 * worse than no color.
 */
const BY_EXTENSION: Record<string, string> = {
  ts: "typescript",
  tsx: "tsx",
  js: "javascript",
  jsx: "jsx",
  mjs: "javascript",
  cjs: "javascript",
  py: "python",
  rs: "rust",
  go: "go",
  rb: "ruby",
  java: "java",
  kt: "kotlin",
  swift: "swift",
  c: "c",
  h: "c",
  cc: "cpp",
  cpp: "cpp",
  hpp: "cpp",
  cs: "csharp",
  php: "php",
  sh: "shell",
  bash: "shell",
  zsh: "shell",
  fish: "shell",
  sql: "sql",
  html: "html",
  css: "css",
  scss: "scss",
  json: "json",
  jsonc: "jsonc",
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  xml: "xml",
  svg: "xml",
  md: "markdown",
  markdown: "markdown",
  lua: "lua",
  vim: "viml",
  dockerfile: "docker",
  makefile: "make",
  ini: "ini",
  diff: "diff",
  patch: "diff",
};

/** Files whose whole name (not extension) determines the grammar. */
const BY_FILENAME: Record<string, string> = {
  dockerfile: "docker",
  makefile: "make",
  ".gitignore": "ini",
  ".env": "ini",
};

/** Fence labels as people actually write them (```py, ```sh, ```rs). */
const BY_FENCE: Record<string, string> = {
  py: "python",
  python: "python",
  rs: "rust",
  rust: "rust",
  ts: "typescript",
  typescript: "typescript",
  tsx: "tsx",
  js: "javascript",
  javascript: "javascript",
  jsx: "jsx",
  sh: "shell",
  bash: "shell",
  zsh: "shell",
  shell: "shell",
  console: "shell",
  go: "go",
  golang: "go",
  rb: "ruby",
  ruby: "ruby",
  java: "java",
  kt: "kotlin",
  kotlin: "kotlin",
  swift: "swift",
  c: "c",
  cpp: "cpp",
  "c++": "cpp",
  cs: "csharp",
  csharp: "csharp",
  php: "php",
  sql: "sql",
  html: "html",
  css: "css",
  scss: "scss",
  json: "json",
  jsonc: "jsonc",
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  xml: "xml",
  md: "markdown",
  markdown: "markdown",
  lua: "lua",
  dockerfile: "docker",
  docker: "docker",
  make: "make",
  makefile: "make",
  ini: "ini",
  diff: "diff",
  patch: "diff",
  vim: "viml",
};

/** Shiki language for a fenced-code label, or null to leave it plain. */
export function languageForFence(label: string): string | null {
  return BY_FENCE[label.trim().toLowerCase()] ?? null;
}

export function languageFor(path: string): string | null {
  const name = path.split("/").pop()?.toLowerCase() ?? "";
  if (BY_FILENAME[name]) return BY_FILENAME[name];
  const ext = name.includes(".") ? name.split(".").pop()! : "";
  return BY_EXTENSION[ext] ?? null;
}

export function currentShikiTheme(): "vitesse-light" | "vitesse-dark" | "vitesse-black" {
  if (typeof document === "undefined") return "vitesse-dark";
  const theme = document.documentElement.dataset.theme;
  if (theme === "light" || theme === "sage" || theme === "paper" || theme === "mist" || theme === "dawn") return "vitesse-light";
  if (theme === "contrast") return "vitesse-black";
  return "vitesse-dark";
}

async function core(): Promise<HighlighterCore> {
  if (!corePromise) {
    corePromise = (async () => {
      const { createHighlighterCore } = await import("shiki/core");
      const { createOnigurumaEngine } = await import("shiki/engine/oniguruma");
      return createHighlighterCore({
        themes: [
          import("@shikijs/themes/vitesse-dark"),
          import("@shikijs/themes/vitesse-light"),
          import("@shikijs/themes/vitesse-black"),
        ],
        langs: [],
        engine: createOnigurumaEngine(import("shiki/wasm")),
      });
    })();
  }
  return corePromise;
}

/**
 * Explicit lazy importers, one per supported grammar.
 *
 * A template-literal import would make Vite glob the whole grammar
 * directory — 270 chunks and ~9MB emitted for languages nobody opens. Naming
 * them keeps the build to the set we actually map, and each is still fetched
 * on demand.
 */
const GRAMMARS: Record<string, () => Promise<unknown>> = {
  typescript: () => import("@shikijs/langs/typescript"),
  tsx: () => import("@shikijs/langs/tsx"),
  javascript: () => import("@shikijs/langs/javascript"),
  jsx: () => import("@shikijs/langs/jsx"),
  python: () => import("@shikijs/langs/python"),
  rust: () => import("@shikijs/langs/rust"),
  go: () => import("@shikijs/langs/go"),
  ruby: () => import("@shikijs/langs/ruby"),
  java: () => import("@shikijs/langs/java"),
  kotlin: () => import("@shikijs/langs/kotlin"),
  swift: () => import("@shikijs/langs/swift"),
  c: () => import("@shikijs/langs/c"),
  cpp: () => import("@shikijs/langs/cpp"),
  csharp: () => import("@shikijs/langs/csharp"),
  php: () => import("@shikijs/langs/php"),
  shell: () => import("@shikijs/langs/shellscript"),
  sql: () => import("@shikijs/langs/sql"),
  html: () => import("@shikijs/langs/html"),
  css: () => import("@shikijs/langs/css"),
  scss: () => import("@shikijs/langs/scss"),
  json: () => import("@shikijs/langs/json"),
  jsonc: () => import("@shikijs/langs/jsonc"),
  yaml: () => import("@shikijs/langs/yaml"),
  toml: () => import("@shikijs/langs/toml"),
  xml: () => import("@shikijs/langs/xml"),
  markdown: () => import("@shikijs/langs/markdown"),
  lua: () => import("@shikijs/langs/lua"),
  docker: () => import("@shikijs/langs/docker"),
  make: () => import("@shikijs/langs/make"),
  ini: () => import("@shikijs/langs/ini"),
  diff: () => import("@shikijs/langs/diff"),
  viml: () => import("@shikijs/langs/viml"),
};

async function ensureLanguage(hl: HighlighterCore, lang: string): Promise<boolean> {
  if (loaded.has(lang)) return true;
  const load = GRAMMARS[lang];
  if (!load) return false;
  try {
    const mod = (await load()) as { default?: unknown };
    await hl.loadLanguage((mod.default ?? mod) as never);
    loaded.add(lang);
    return true;
  } catch {
    // An unavailable grammar is not an error worth failing the render for.
    return false;
  }
}

/**
 * Highlight `code` as `lang`, returning HTML. Returns null when the language
 * is unknown or the grammar cannot be loaded, so callers render plain text
 * rather than showing nothing.
 */
export async function highlight(
  code: string,
  lang: string | null,
  overrideTheme?: string,
): Promise<string | null> {
  if (!lang) return null;
  try {
    const hl = await core();
    if (!(await ensureLanguage(hl, lang))) return null;
    const theme = overrideTheme ?? currentShikiTheme();
    return hl.codeToHtml(code, { lang, theme });
  } catch {
    return null;
  }
}
