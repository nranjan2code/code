import { For, Show } from "solid-js";
import { safeUrl } from "../../safeUrl";

const fieldLabel = (key: string) => key
  .replace(/[_-]+/g, " ")
  .replace(/\b\w/g, (letter) => letter.toUpperCase());

export function PresentationValue(props: { value: unknown }) {
  const value = () => props.value;
  return (
    <Show when={value() !== null && value() !== undefined && value() !== ""} fallback={<span class="universal-card-empty">Unavailable</span>}>
      <Show when={Array.isArray(value())} fallback={
        <Show when={typeof value() === "object"} fallback={
          <Show when={typeof value() === "string" && safeUrl(value() as string)} fallback={<>{typeof value() === "boolean" ? (value() ? "Yes" : "No") : String(value())}</>}>
            <a href={value() as string} target="_blank" rel="noreferrer noopener">{value() as string}</a>
          </Show>
        }>
          <dl class="universal-card-nested">
            <For each={Object.entries(value() as Record<string, unknown>)}>{([key, nested]) => (
              <div><dt>{fieldLabel(key)}</dt><dd><PresentationValue value={nested} /></dd></div>
            )}</For>
          </dl>
        </Show>
      }>
        <ul class="universal-card-list">
          <For each={value() as unknown[]}>{(item) => <li><PresentationValue value={item} /></li>}</For>
        </ul>
      </Show>
    </Show>
  );
}

/** Conservative renderer for universal semantic shapes. Domain-specific
 * renderers may replace this later without changing the protocol or seeds. */
export default function UniversalCard(props: { data: Record<string, any>; kind: string }) {
  const entries = () => Object.entries(props.data ?? {}).filter(([key]) => !["semantic_type", "title", "summary"].includes(key));
  return (
    <section class="canvas-card universal-card" aria-label={`${props.kind} result`}>
      <Show when={props.data?.title}><h3>{String(props.data.title)}</h3></Show>
      <Show when={props.data?.summary}><p>{String(props.data.summary)}</p></Show>
      <dl>
        <For each={entries()}>{([key, value]) => (
          <div class="universal-card-row">
            <dt>{fieldLabel(key)}</dt>
            <dd><PresentationValue value={value} /></dd>
          </div>
        )}</For>
      </dl>
    </section>
  );
}
