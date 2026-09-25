import { For } from "solid-js";

/** The one loading placeholder (docs/design/75 §5.4): shimmering shapes in
 * place of "Loading…" text, announced once to assistive technology. */
export type SkeletonShape = "wide" | "medium" | "short" | "card" | "row" | "block";

const SHAPES: Record<string, SkeletonShape[]> = {
  transcript: ["wide", "medium", "card", "wide", "short"],
  text: ["wide", "medium", "short"],
  rows: ["row", "row", "row"],
  blocks: ["block", "block", "block"],
};

export default function Skeleton(props: { label: string; kind?: keyof typeof SHAPES; shapes?: SkeletonShape[]; class?: string }) {
  const shapes = () => props.shapes ?? SHAPES[props.kind ?? "text"];
  return (
    <div class={`skeleton${props.class ? ` ${props.class}` : ""}`} role="status" aria-label={props.label}>
      <For each={shapes()}>{(shape) => <span class={`skeleton-${shape}`} aria-hidden="true" />}</For>
    </div>
  );
}
