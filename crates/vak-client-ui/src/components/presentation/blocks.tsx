import { For, Show } from "solid-js";
import type { AdaptiveRenderNode } from "../../types";

// Composed-answer blocks (docs/design/87-composed-answers.md). Each builder
// turns a stored payload into a render node; each renderer draws one block.

type Rec = Record<string, unknown>;
const rec = (data: unknown): Rec => (data && typeof data === "object" && !Array.isArray(data) ? (data as Rec) : {});
const text = (value: unknown): string | undefined =>
  typeof value === "string" ? value : typeof value === "number" ? String(value) : undefined;
const texts = (value: unknown): string[] =>
  Array.isArray(value) ? value.map(text).filter((item): item is string => !!item && item.trim().length > 0) : [];
const node = (primitive: string, props: Rec): AdaptiveRenderNode => ({ primitive, props, children: [] });

export const buildLeadSpec = (data: unknown) => {
  const r = rec(data);
  return node("lead", { label: text(r.label), headline: text(r.headline) ?? "", text: text(r.text), points: texts(r.points) });
};

export const buildStatsSpec = (data: unknown) => {
  const r = rec(data);
  const readings = Array.isArray(r.readings)
    ? r.readings.map(rec).map((reading) => ({
        label: text(reading.label) ?? "",
        value: text(reading.value) ?? "—",
        unit: text(reading.unit),
        change: text(reading.change),
      }))
    : [];
  return node("stats", { label: text(r.label), title: text(r.title), readings });
};

export const buildCalloutSpec = (data: unknown) => {
  const r = rec(data);
  const tone = ["tip", "note", "warning", "important"].includes(String(r.tone)) ? String(r.tone) : "note";
  return node("callout", { label: text(r.label), tone, title: text(r.title), text: text(r.text) ?? "" });
};

export const buildChecklistSpec = (data: unknown) => {
  const r = rec(data);
  const items = (Array.isArray(r.items) ? r.items : [])
    .map((item) => (typeof item === "string" ? { label: item } : { label: text(rec(item).label) ?? "", detail: text(rec(item).detail) }))
    .filter((item) => item.label);
  return node("checklist", { label: text(r.label), title: text(r.title), items });
};

/** A comparison stored with `options`; older comparisons were tables and
 * keep their table renderer (the caller checks first). */
export const isOptionsComparison = (data: unknown) => Array.isArray(rec(data).options);

export const buildComparisonSpec = (data: unknown) => {
  const r = rec(data);
  const options = (Array.isArray(r.options) ? r.options : []).map(rec).map((option) => ({
    name: text(option.name) ?? "",
    verdict: text(option.verdict),
    strengths: texts(option.strengths),
    weaknesses: texts(option.weaknesses),
  }));
  const aspects = (Array.isArray(r.aspects) ? r.aspects : []).map(rec).map((aspect) => ({
    label: text(aspect.label) ?? "",
    values: texts(aspect.values),
  }));
  return node("comparison_options", {
    label: text(r.label),
    title: text(r.title),
    options,
    aspects,
    recommendation: text(r.recommendation),
  });
};

const Eyebrow = (props: { label?: unknown }) => (
  <Show when={text(props.label)}>{(label) => <div class="block-eyebrow">{label()}</div>}</Show>
);

export function renderLead(n: AdaptiveRenderNode) {
  const points = n.props.points as string[];
  return (
    <section class="answer-block block-lead" aria-label="Answer">
      <Eyebrow label={n.props.label} />
      <h3 class="block-lead-headline">{text(n.props.headline)}</h3>
      <Show when={text(n.props.text)}>{(body) => <p class="block-lead-text">{body()}</p>}</Show>
      <Show when={points.length > 0}>
        <ul class="block-lead-points"><For each={points}>{(point) => <li>{point}</li>}</For></ul>
      </Show>
    </section>
  );
}

export function renderStats(n: AdaptiveRenderNode) {
  const readings = n.props.readings as { label: string; value: string; unit?: string; change?: string }[];
  return (
    <section class="answer-block block-stats" aria-label={text(n.props.title) ?? "Key figures"}>
      <Eyebrow label={n.props.label ?? n.props.title} />
      <div class="block-stats-row">
        <For each={readings}>{(reading) => (
          <div class="block-stat">
            <div class="block-stat-value">{reading.value}<Show when={reading.unit}><span class="block-stat-unit">{reading.unit}</span></Show></div>
            <div class="block-stat-label">{reading.label}</div>
            <Show when={reading.change}><div class="block-stat-change">{reading.change}</div></Show>
          </div>
        )}</For>
      </div>
    </section>
  );
}

export function renderCallout(n: AdaptiveRenderNode) {
  const tone = String(n.props.tone);
  const names: Record<string, string> = { tip: "Tip", note: "Note", warning: "Warning", important: "Important" };
  return (
    <aside class={`answer-block block-callout tone-${tone}`} role={tone === "warning" || tone === "important" ? "alert" : "note"}>
      <div class="block-callout-tone">{text(n.props.label) ?? names[tone]}</div>
      <Show when={text(n.props.title)}>{(title) => <strong class="block-callout-title">{title()}</strong>}</Show>
      <p>{text(n.props.text)}</p>
    </aside>
  );
}

export function renderChecklist(n: AdaptiveRenderNode) {
  const items = n.props.items as { label: string; detail?: string }[];
  return (
    <section class="answer-block block-checklist" aria-label={text(n.props.title) ?? "Checklist"}>
      <Eyebrow label={n.props.label} />
      <Show when={text(n.props.title)}>{(title) => <h3>{title()}</h3>}</Show>
      <ul>
        <For each={items}>{(item) => (
          <li><label><input type="checkbox" /> <span>{item.label}</span></label><Show when={item.detail}><small>{item.detail}</small></Show></li>
        )}</For>
      </ul>
    </section>
  );
}

export function renderComparisonOptions(n: AdaptiveRenderNode) {
  const options = n.props.options as { name: string; verdict?: string; strengths: string[]; weaknesses: string[] }[];
  const aspects = n.props.aspects as { label: string; values: string[] }[];
  return (
    <section class="answer-block block-comparison" aria-label={text(n.props.title) ?? "Comparison"}>
      <Eyebrow label={n.props.label} />
      <Show when={text(n.props.title)}>{(title) => <h3>{title()}</h3>}</Show>
      <div class="block-comparison-options" style={{ "--options": String(options.length) }}>
        <For each={options}>{(option, index) => (
          <article class="block-option">
            <h4>{option.name}</h4>
            <Show when={option.verdict}><p class="block-option-verdict">{option.verdict}</p></Show>
            <Show when={option.strengths.length > 0}>
              <ul class="block-option-strengths" aria-label="Strengths"><For each={option.strengths}>{(point) => <li>{point}</li>}</For></ul>
            </Show>
            <Show when={option.weaknesses.length > 0}>
              <ul class="block-option-weaknesses" aria-label="Weaknesses"><For each={option.weaknesses}>{(point) => <li>{point}</li>}</For></ul>
            </Show>
            <Show when={aspects.length > 0}>
              <dl class="block-option-aspects">
                <For each={aspects}>{(aspect) => (
                  <div><dt>{aspect.label}</dt><dd>{aspect.values[index()] ?? "—"}</dd></div>
                )}</For>
              </dl>
            </Show>
          </article>
        )}</For>
      </div>
      <Show when={text(n.props.recommendation)}>{(recommendation) => <p class="block-comparison-recommendation">{recommendation()}</p>}</Show>
    </section>
  );
}
