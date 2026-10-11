/** Every theme a person can choose, in the order Settings lists them, by the
 *  name it shows (DESIGN.md, Themes). */
export const themeChoices = [
  { id: "system", label: "Match system" },
  { id: "light", label: "Light" },
  { id: "dark", label: "Dark" },
  { id: "contrast", label: "High contrast" },
  { id: "frosted-light", label: "Frosted light" },
  { id: "frosted-dark", label: "Frosted dark" },
] as const;
export type ThemeChoice = (typeof themeChoices)[number]["id"];
/** The palette in force: every choice but "system", which resolves to one. */
export type ResolvedTheme = Exclude<ThemeChoice, "system">;

export function isThemeChoice(value: unknown): value is ThemeChoice {
  return themeChoices.some((choice) => choice.id === value);
}

export function resolveTheme(choice: ThemeChoice, systemDark: boolean): ResolvedTheme {
  if (choice === "system") return systemDark ? "dark" : "light";
  return choice;
}

/** Whether a palette is a light one, for everything that has only a light
 *  and a dark form (the colour scheme, code highlighting). */
export function themeIsLight(theme: string | undefined): boolean {
  return theme === "light" || theme === "frosted-light";
}
