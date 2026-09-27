// The one line a file card says about an Office file or PDF (docs/design/72
// U6, 77): what it is and its headline counts, from the reader's own stats.

type Facts = {
  vocabulary: "word" | "excel" | "power_point" | "visio" | "pdf";
  kind: string;
  stats: [string, number][];
  flags: string[];
};

const HEADLINE: Record<Facts["vocabulary"], string[]> = {
  word: ["words", "tracked changes", "comments"],
  excel: ["sheets", "formulas"],
  power_point: ["slides", "hidden slides"],
  visio: ["pages", "shapes"],
  pdf: ["pages", "comments", "form fields"],
};

/** "3 slides", "1 slide", "1 hidden slide", "1 slide with notes". */
export function countOf(name: string, value: number): string {
  const words = name.split(" ");
  const qualifier = words.indexOf("with");
  const noun = qualifier > 0 ? qualifier - 1 : words.length - 1;
  if (value === 1 && words[noun].endsWith("s")) words[noun] = words[noun].slice(0, -1);
  return `${value.toLocaleString("en-US")} ${words.join(" ")}`;
}

/** "PowerPoint presentation · 3 slides", "Word document · 1,240 words · 2 tracked changes". */
export function officeFactsLine(facts: Facts): string {
  const stats = new Map(facts.stats);
  const counts = HEADLINE[facts.vocabulary]
    .filter((name) => (stats.get(name) ?? 0) > 0 || name === HEADLINE[facts.vocabulary][0])
    .slice(0, 3)
    .map((name) => countOf(name, stats.get(name) ?? 0));
  return [facts.kind, ...counts].join(" · ");
}

/** "1 flag", "3 flags", or nothing. */
export function officeFlagsLabel(facts: Facts): string | null {
  const n = facts.flags.length;
  return n === 0 ? null : `${n} ${n === 1 ? "flag" : "flags"}`;
}
