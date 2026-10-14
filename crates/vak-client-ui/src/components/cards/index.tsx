import { For, Show, type JSX } from "solid-js";
import type { AdaptiveRenderNode } from "../../types";
import ChartCard from "./Chart";
import { DiffForm, LinkForm, MediaForm, PreviewForm, TerminalForm, TestReportForm } from "./code";
import {
  CalendarForm, CalloutForm, ComparisonForm, DetailsForm, ListForm, SourcesForm, StatsForm, StepsForm,
  SummaryForm, TableForm,
} from "./forms";
import { record, text, type Payload } from "./shared";

export { PresentationInteractionContext, PresentationSessionContext, downloadCsv } from "./shared";

/** One renderer per form (docs/design/92-card-forms.md), keyed by the card's
 *  block. A kind is never read here: what draws a card is its form. */
const FORMS: Record<string, (props: { payload: Payload }) => JSX.Element> = {
  summary: SummaryForm,
  steps: StepsForm,
  list: ListForm,
  details: DetailsForm,
  table: TableForm,
  comparison: ComparisonForm,
  stats: StatsForm,
  chart: ChartCard,
  callout: CalloutForm,
  sources: SourcesForm,
  calendar: CalendarForm,
  link: LinkForm,
  media: MediaForm,
  file_preview: PreviewForm,
  markup_preview: PreviewForm,
  diff: DiffForm,
  test_report: TestReportForm,
  terminal: TerminalForm,
};

/** The forms this client draws, checked against the host's list in tests. */
export const formNames = Object.freeze(Object.keys(FORMS));

/** A card by its form, or its own text when no form draws it (a plugin
 *  skill's kind, or a card recorded before its form existed). */
export function FormView(props: { block: string; payload: Payload; fallback: string }) {
  const form = () => FORMS[props.block];
  return (
    <Show when={form()} fallback={<pre class="card-text">{props.fallback}</pre>}>
      {(draw) => draw()({ payload: props.payload })}
    </Show>
  );
}

/** A pack's render tree: layout arranges its parts; a form node draws its
 *  form from its props, which are that form's payload. */
export function PackTree(props: { node: AdaptiveRenderNode }): JSX.Element {
  const node = () => props.node;
  const p = () => record(node().props);
  const children = () => (Array.isArray(node().children) ? node().children : []);
  const parts = () => <For each={children()}>{(child) => <PackTree node={child} />}</For>;
  switch (node().primitive) {
    case "stack":
    case "row":
    case "group":
      return <div class={`pack-${node().primitive}`}>{parts()}</div>;
    case "section":
      return (
        <section class="pack-section" aria-label={text(p().title) ?? text(p().label)}>
          <Show when={text(p().label)}>{(label) => <span class="card-eyebrow">{label()}</span>}</Show>
          <Show when={text(p().title)}>{(title) => <h3 class="card-title">{title()}</h3>}</Show>
          <Show when={text(p().text)}>{(body) => <p class="card-lede">{body()}</p>}</Show>
          {parts()}
        </section>
      );
    case "text":
      return <p class="pack-text">{text(p().text)}</p>;
    case "metric":
      return (
        <div class="stat">
          <div class="stat-value">{text(p().value) ?? "—"}<Show when={text(p().unit)}>{(unit) => <span class="stat-unit">{unit()}</span>}</Show></div>
          <div class="stat-label">{text(p().label)}</div>
          <Show when={text(p().change)}>{(change) => <div class="stat-change">{change()}</div>}</Show>
        </div>
      );
    default:
      return <FormView block={node().primitive} payload={p()} fallback={text(p().text) ?? ""} />;
  }
}
