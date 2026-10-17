import { Show, createContext, type JSX } from "solid-js";
import { renderMarkdown } from "../../md";

/** A stored card payload: the form's fields, as the host built them. */
export type Payload = Record<string, unknown>;

/** What a person can do on a card, recorded as card actions
 *  (docs/design/96, Q3): choose an option, which starts a turn, or tick an
 *  item, whose state `marked` holds. */
export const PresentationInteractionContext = createContext<{
  onOptionSelect(label: string): void;
  marked: number[];
  onMark(index: number, label: string, checked: boolean): Promise<boolean>;
  /** What the person set on a controls card's inputs, by name; a turn follows. */
  onValues?(values: Record<string, string>): void;
}>();
/** The conversation a card belongs to: a preview reads its files from that
 *  conversation's folder, which is its Agent's own. */
export const PresentationSessionContext = createContext<string | undefined>();

export const record = (value: unknown): Payload =>
  value && typeof value === "object" && !Array.isArray(value) ? (value as Payload) : {};

export const text = (value: unknown): string | undefined => {
  if (typeof value === "string") return value.trim() || undefined;
  if (typeof value === "number" && Number.isFinite(value)) return String(value);
  return undefined;
};

export const num = (value: unknown): number | undefined =>
  typeof value === "number" && Number.isFinite(value) ? value : undefined;

export const records = (value: unknown): Payload[] => (Array.isArray(value) ? value.map(record) : []);

export const texts = (value: unknown): string[] =>
  Array.isArray(value) ? value.map(text).filter((item): item is string => !!item) : [];

/** One card: the same shell for every form. The eyebrow is the model's own
 *  words for this part of the answer; the title names the content. */
export function Card(props: {
  form: string;
  label?: unknown;
  title?: unknown;
  actions?: JSX.Element;
  children: JSX.Element;
  tag?: "section" | "aside";
}) {
  const header = () => text(props.label) || text(props.title) || props.actions;
  const body = (
    <>
      <Show when={header()}>
        <header class="card-head">
          <div class="card-heading">
            <Show when={text(props.label)}>{(label) => <span class="card-eyebrow">{label()}</span>}</Show>
            <Show when={text(props.title)}>{(title) => <h3 class="card-title">{title()}</h3>}</Show>
          </div>
          <Show when={props.actions}><div class="card-actions">{props.actions}</div></Show>
        </header>
      </Show>
      {props.children}
    </>
  );
  return props.tag === "aside"
    ? <aside class={`card card-${props.form}`}>{body}</aside>
    : <section class={`card card-${props.form}`} aria-label={text(props.title) ?? text(props.label)}>{body}</section>;
}

export function csvCell(value: unknown): string {
  const cell = value == null ? "" : String(value);
  const safe = typeof value === "string" && /^[=+@\-\t\r]/.test(cell) ? `'${cell}` : cell;
  return `"${safe.replace(/"/g, '""')}"`;
}

export function downloadCsv(filename: string, rows: unknown[][]): void {
  const content = rows.map((row) => row.map(csvCell).join(",")).join("\r\n");
  const url = URL.createObjectURL(new Blob([content], { type: "text/csv;charset=utf-8" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

/** A file name for an export, from the card's own heading. */
export function exportName(title: string | undefined, fallback: string): string {
  const base = (title ?? fallback).toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
  return `${base || fallback}.csv`;
}

/** A card's prose field, read as the conversation's own text is: a link or
 *  emphasis the model wrote shows as one, never as its markup. */
export function Prose(props: { class: string; text: string }): JSX.Element {
  return <div class={`card-prose ${props.class}`} innerHTML={renderMarkdown(props.text)} />;
}
