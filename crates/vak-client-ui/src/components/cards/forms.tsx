import { For, Show, createMemo, createSignal, useContext } from "solid-js";
import { safeUrl } from "../../safeUrl";
import { Card, PresentationInteractionContext, Prose, downloadCsv, exportName, record, records, text, texts, type Payload } from "./shared";
import { sortTableRows } from "./tableNumbers";

/** The answer itself: prose on the page, not a card (DESIGN.md "Cards and results"). */
export function SummaryForm(props: { payload: Payload }) {
  const p = () => props.payload;
  return (
    <section class="answer-summary" aria-label="Answer">
      <Show when={text(p().label)}>{(label) => <span class="card-eyebrow">{label()}</span>}</Show>
      <h2 class="answer-headline">{text(p().headline)}</h2>
      <Show when={text(p().text)}>{(body) => <Prose class="answer-text" text={body()} />}</Show>
      <Show when={texts(p().points).length > 0}>
        <ul class="answer-points"><For each={texts(p().points)}>{(point) => <li>{point}</li>}</For></ul>
      </Show>
    </section>
  );
}

function Linked(props: { label: string; url?: string }) {
  return (
    <Show when={props.url && safeUrl(props.url)} fallback={<>{props.label}</>}>
      <a href={props.url} target="_blank" rel="noreferrer noopener">{props.label}</a>
    </Show>
  );
}

/** Things done in order. The time, when there is one, leads its step. */
export function StepsForm(props: { payload: Payload }) {
  const p = () => props.payload;
  const interaction = useContext(PresentationInteractionContext);
  return (
    <Card form="steps" label={p().label} title={p().title}>
      <ol class="steps">
        <For each={records(p().steps)}>{(step) => (
          <li class="step">
            <div class="step-body">
              <div class="step-head">
                <Show when={text(step.time)}>{(time) => <span class="step-time">{time()}</span>}</Show>
                <span class="step-label">{text(step.label)}</span>
              </div>
              <Show when={text(step.detail)}>{(detail) => <Prose class="step-detail" text={detail()} />}</Show>
              <Show when={records(step.choices).length > 0}>
                <ul class="step-choices" aria-label="Choices">
                  <For each={records(step.choices)}>{(choice) => (
                    <li class="step-choice">
                      <div>
                        <span class="step-choice-label">{text(choice.label)}</span>
                        <Show when={text(choice.detail)}>{(detail) => <span class="card-meta"> · {detail()}</span>}</Show>
                        <Show when={texts(choice.facts).length > 0}><div class="card-meta">{texts(choice.facts).join(" · ")}</div></Show>
                      </div>
                      <Show when={interaction}>
                        <button type="button" class="card-button" onClick={() => interaction?.onOptionSelect(text(choice.label) ?? "")}>Use this</button>
                      </Show>
                    </li>
                  )}</For>
                </ul>
              </Show>
            </div>
          </li>
        )}</For>
      </ol>
    </Card>
  );
}

/** Items in no order; ticked off when the card says they are to be. */
export function ListForm(props: { payload: Payload }) {
  const p = () => props.payload;
  const checkable = () => p().checkable === true;
  const interaction = useContext(PresentationInteractionContext);
  // Ticks are card actions (docs/design/96, Q3): drawn from the card's
  // state, changed here at once, and put back if the change is refused.
  const [marked, setMarked] = createSignal<number[]>(interaction?.marked ?? []);
  const toggle = (index: number, label: string, checked: boolean) => {
    const before = marked();
    setMarked(checked ? [...before.filter((at) => at !== index), index] : before.filter((at) => at !== index));
    if (interaction) void interaction.onMark(index, label, checked).then((recorded) => { if (!recorded) setMarked(before); });
  };
  return (
    <Card form="list" label={p().label} title={p().title}>
      <ul class="list" classList={{ "is-checkable": checkable() }}>
        <For each={records(p().items)}>{(item, index) => {
          const body = (
            <span class="list-body">
              <span class="list-label"><Linked label={text(item.label) ?? ""} url={text(item.url)} /></span>
              <Show when={text(item.detail)}>{(detail) => <Prose class="list-detail" text={detail()} />}</Show>
            </span>
          );
          return <li><Show when={checkable()} fallback={body}><label class="list-check"><input type="checkbox" checked={marked().includes(index())} onChange={(event) => toggle(index(), text(item.label) ?? "", event.currentTarget.checked)} />{body}</label></Show></li>;
        }}</For>
      </ul>
    </Card>
  );
}

/** Labelled facts about one subject, or questions with their answers. */
export function DetailsForm(props: { payload: Payload }) {
  const p = () => props.payload;
  return (
    <Card form="details" label={p().label} title={p().title}>
      <Show when={text(p().text)}>{(body) => <Prose class="card-lede" text={body()} />}</Show>
      <dl class="details">
        <For each={records(p().fields)}>{(field) => (
          <div class="details-row"><dt>{text(field.label)}</dt><dd><Prose class="details-value" text={text(field.detail) ?? ""} /></dd></div>
        )}</For>
      </dl>
    </Card>
  );
}

/** Records under shared columns: sortable, never scrolled inside the page. */
export function TableForm(props: { payload: Payload }) {
  const p = () => props.payload;
  const columns = () => records(p().columns).map((column) => ({ label: text(column.label) ?? "", numeric: column.numeric === true }));
  const rows = () => (Array.isArray(p().rows) ? (p().rows as unknown[]).filter(Array.isArray) as unknown[][] : []);
  const [sort, setSort] = createSignal<{ column: number; ascending: boolean } | null>(null);
  const sorted = createMemo(() => {
    const order = sort();
    if (!order) return rows();
    const numeric = columns()[order.column]?.numeric;
    return sortTableRows(rows(), order.column, numeric, order.ascending);
  });
  const toggle = (column: number) =>
    setSort((current) => (current?.column === column ? { column, ascending: !current.ascending } : { column, ascending: true }));
  const shown = (value: unknown) => (typeof value === "number" ? new Intl.NumberFormat().format(value) : String(value ?? ""));
  return (
    <Card form="table" label={p().label} title={p().title}
      actions={<button type="button" class="card-button" onClick={() => downloadCsv(exportName(text(p().title), "table"), [columns().map((c) => c.label), ...rows()])}>Download CSV</button>}>
      <div class="table-frame">
        <table class="card-table">
          <thead>
            <tr>
              <For each={columns()}>{(column, index) => (
                <th scope="col" classList={{ "is-numeric": column.numeric }}
                  aria-sort={sort()?.column === index() ? (sort()!.ascending ? "ascending" : "descending") : "none"}>
                  <button type="button" class="table-sort" onClick={() => toggle(index())}>{column.label}</button>
                </th>
              )}</For>
            </tr>
          </thead>
          <tbody>
            <For each={sorted()}>{(row) => (
              <tr><For each={columns()}>{(column, index) => <td classList={{ "is-numeric": column.numeric }}>{shown(row[index()])}</td>}</For></tr>
            )}</For>
          </tbody>
        </table>
      </div>
    </Card>
  );
}

/** Two to four options side by side, with the recommendation under them. */
export function ComparisonForm(props: { payload: Payload }) {
  const p = () => props.payload;
  const options = () => records(p().options);
  const aspects = () => records(p().aspects);
  return (
    <Card form="comparison" label={p().label} title={p().title}>
      <div class="options" style={{ "--options": String(options().length) }}>
        <For each={options()}>{(option, index) => (
          <article class="option">
            <h4 class="option-name">{text(option.name)}</h4>
            <Show when={text(option.verdict)}>{(verdict) => <p class="option-verdict">{verdict()}</p>}</Show>
            <Show when={texts(option.strengths).length > 0}>
              <ul class="option-points is-strength" aria-label="Strengths"><For each={texts(option.strengths)}>{(point) => <li>{point}</li>}</For></ul>
            </Show>
            <Show when={texts(option.weaknesses).length > 0}>
              <ul class="option-points is-weakness" aria-label="Weaknesses"><For each={texts(option.weaknesses)}>{(point) => <li>{point}</li>}</For></ul>
            </Show>
            <Show when={aspects().length > 0}>
              <dl class="details">
                <For each={aspects()}>{(aspect) => (
                  <div class="details-row"><dt>{text(aspect.label)}</dt><dd>{texts(aspect.values)[index()] ?? "—"}</dd></div>
                )}</For>
              </dl>
            </Show>
          </article>
        )}</For>
      </div>
      <Show when={text(p().recommendation)}>{(recommendation) => <Prose class="option-recommendation" text={recommendation()} />}</Show>
    </Card>
  );
}

/** A figure as written; a bare number gets the reader's digit grouping. */
export const figure = (value: unknown) =>
  typeof value === "number" ? new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 }).format(value) : text(value) ?? "—";

/** A few key figures shown large. */
export function StatsForm(props: { payload: Payload }) {
  const p = () => props.payload;
  return (
    <Card form="stats" label={p().label} title={p().title}>
      <div class="stats">
        <For each={records(p().readings)}>{(reading) => (
          <div class="stat">
            <div class="stat-value">{figure(reading.value)}<Show when={text(reading.unit)}>{(unit) => <span class="stat-unit">{unit()}</span>}</Show></div>
            <div class="stat-label">{text(reading.label)}</div>
            <Show when={text(reading.change)}>{(change) => <div class="stat-change">{change()}</div>}</Show>
          </div>
        )}</For>
      </div>
    </Card>
  );
}

/** One note set apart; its tone says how much it matters. */
export function CalloutForm(props: { payload: Payload }) {
  const p = () => props.payload;
  const tone = () => text(p().tone) ?? "note";
  const names: Record<string, string> = { tip: "Tip", note: "Note", warning: "Warning", important: "Important" };
  return (
    <aside class={`callout tone-${tone()}`} role={tone() === "warning" || tone() === "important" ? "alert" : "note"}>
      <span class="callout-tone">{text(p().label) ?? names[tone()] ?? "Note"}</span>
      <Show when={text(p().title)}>{(title) => <strong class="callout-title">{title()}</strong>}</Show>
      <Prose class="callout-text" text={text(p().text) ?? ""} />
    </aside>
  );
}

function hostOf(url: string): string {
  try { return new URL(url).hostname.replace(/^www\./, ""); } catch { return ""; }
}

/** Raw addresses are destinations, not useful source titles. */
function sourceLabel(source: Payload, url: string): string {
  for (const candidate of [text(source.title), text(source.source_name)]) {
    if (candidate && candidate !== url && !/^(?:https?:\/\/|www\.)/i.test(candidate)) return candidate;
  }
  return hostOf(url) || "Source";
}

function readableDate(value?: string): string | undefined {
  if (!value) return undefined;
  const day = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  const date = day ? new Date(Number(day[1]), Number(day[2]) - 1, Number(day[3])) : new Date(value);
  return Number.isFinite(date.getTime()) ? new Intl.DateTimeFormat(undefined, { dateStyle: "medium" }).format(date) : value;
}

/** Findings, each with the source it came from: who said it, when, where. */
export function SourcesForm(props: { payload: Payload }) {
  const p = () => props.payload;
  return (
    <Card form="sources" label={p().label} title={p().title}>
      <ol class="sources">
        <For each={records(p().sources)}>{(source) => {
          const url = text(source.url) ?? "";
          const label = sourceLabel(source, url);
          const sourceName = text(source.source_name);
          const publisher = sourceName && !/^(?:https?:\/\/|www\.)/i.test(sourceName) ? sourceName : hostOf(url);
          const detail = [publisher !== label ? publisher : undefined, readableDate(text(source.published_at))].filter(Boolean).join(" · ");
          return (
            <li class="source">
              <Prose class="source-finding" text={text(source.finding) ?? ""} />
              <div class="source-from">
                <Linked label={label} url={url} />
                <Show when={detail}><span class="card-meta">{detail}</span></Show>
              </div>
            </li>
          );
        }}</For>
      </ol>
    </Card>
  );
}

/** Events on their days, in time order. */
export function CalendarForm(props: { payload: Payload }) {
  const p = () => props.payload;
  const days = createMemo(() => {
    const byDay = new Map<string, Payload[]>();
    for (const event of records(p().events)) {
      const date = text(event.date) ?? "";
      byDay.set(date, [...(byDay.get(date) ?? []), event]);
    }
    return [...byDay.entries()];
  });
  const dayName = (value: string) => {
    const day = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
    if (!day) return value;
    const date = new Date(Number(day[1]), Number(day[2]) - 1, Number(day[3]));
    return new Intl.DateTimeFormat(undefined, { weekday: "long", day: "numeric", month: "long" }).format(date);
  };
  return (
    <Card form="calendar" label={p().label} title={p().title}>
      <div class="calendar">
        <For each={days()}>{([date, events]) => (
          <section class="calendar-day" aria-label={dayName(date)}>
            <h4 class="calendar-date">{dayName(date)}</h4>
            <ul class="calendar-events">
              <For each={events}>{(event) => (
                <li class="calendar-event">
                  <span class="calendar-time">{[text(event.start), text(event.end)].filter(Boolean).join("–") || "All day"}</span>
                  <span class="calendar-what">
                    <span class="calendar-title">{text(event.title)}</span>
                    <Show when={text(event.location) || text(event.detail)}>
                      <span class="card-meta">{[text(event.location), text(event.detail)].filter(Boolean).join(" · ")}</span>
                    </Show>
                  </span>
                </li>
              )}</For>
            </ul>
          </section>
        )}</For>
      </div>
    </Card>
  );
}

export { record };
