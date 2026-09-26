export const interfaceFonts = {
  system: { label: "System", stack: '-apple-system, BlinkMacSystemFont, "Segoe UI", Ubuntu, Cantarell, sans-serif' },
  arial: { label: "Arial", stack: 'Arial, "Liberation Sans", sans-serif' },
  verdana: { label: "Verdana", stack: 'Verdana, "DejaVu Sans", sans-serif' },
  trebuchet: { label: "Trebuchet", stack: '"Trebuchet MS", "Liberation Sans", sans-serif' },
  georgia: { label: "Georgia", stack: 'Georgia, "Liberation Serif", serif' },
  newsreader: { label: "Newsreader", stack: '"Newsreader", Georgia, serif' },
} as const;

export const contentFonts = {
  inherit: { label: "Same as interface", stack: "" },
  ...interfaceFonts,
} as const;

export const codeFonts = {
  system: { label: "System monospace", stack: '"SFMono-Regular", "SF Mono", ui-monospace, Menlo, Consolas, monospace' },
  jetbrains: { label: "JetBrains Mono", stack: '"JetBrains Mono", "DejaVu Sans Mono", ui-monospace, monospace' },
  fira: { label: "Fira Code", stack: '"Fira Code", "DejaVu Sans Mono", ui-monospace, monospace' },
  menlo: { label: "Menlo", stack: 'Menlo, "DejaVu Sans Mono", ui-monospace, monospace' },
  consolas: { label: "Consolas", stack: 'Consolas, "Liberation Mono", ui-monospace, monospace' },
} as const;

export type InterfaceFont = keyof typeof interfaceFonts;
export type ContentFont = keyof typeof contentFonts;
export type CodeFont = keyof typeof codeFonts;
