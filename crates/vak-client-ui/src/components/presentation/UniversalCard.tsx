import { For, Show } from "solid-js";

/** Conservative renderer for universal semantic shapes. Domain-specific
 * renderers may replace this later without changing the protocol or seeds. */
export default function UniversalCard(props: { data: Record<string, any>; kind: string }) {
  const entries = () => Object.entries(props.data ?? {}).filter(([key]) => key !== "semantic_type" && key !== "title");
  return (
    <section class="canvas-card universal-card" aria-label={`${props.kind} result`}>
      <Show when={props.data?.title}><h3>{String(props.data.title)}</h3></Show>
      <Show when={props.data?.summary}><p>{String(props.data.summary)}</p></Show>
      <dl>
        <For each={entries()}>{([key, value]) => (
          <div class="universal-card-row">
            <dt>{key.replace(/_/g, " ")}</dt>
            <dd>{typeof value === "object" ? JSON.stringify(value) : String(value)}</dd>
          </div>
        )}</For>
      </dl>
    </section>
  );
}
