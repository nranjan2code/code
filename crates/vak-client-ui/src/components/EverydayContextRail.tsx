import { For, Show, createMemo } from "solid-js";
import { activeId, itemsOf, presentationOf, presentationMode } from "../store";
import Icon from "./Icon";

/**
 * The Everyday companion rail is intentionally data-shaped, not scenario-shaped.
 * It gives the simple mode a stable place for useful context while allowing the
 * presentation runtime to populate it for any domain (or leave it empty).
 */
export default function EverydayContextRail() {
  const task = createMemo(() => activeId());
  const timeline = createMemo(() => presentationOf(task()));
  const files = createMemo(() => itemsOf(task()).filter((item) => item.kind === "tool").length);
  const notes = createMemo(() => timeline()?.items.length ?? 0);
  const nextSteps = createMemo(() => timeline()?.goal?.additions.length ?? 0);
  const hasContext = createMemo(() => !!task() && (files() > 0 || notes() > 0 || nextSteps() > 0));

  return (
    <Show when={presentationMode() === "everyday"}>
      <aside class="everyday-rail" aria-label="Helpful details">
        <div class="everyday-rail-head">
          <div>
            <h2>Helpful details</h2>
            <p>Context related to this conversation</p>
          </div>
          <Icon name="close" />
        </div>
        <Show when={hasContext()} fallback={
          <div class="everyday-rail-empty">
            <div class="everyday-rail-empty-icon"><Icon name="file" /></div>
            <strong>Things related to your conversation will appear here.</strong>
            <span>Files, notes, and next steps stay close without crowding the conversation.</span>
          </div>
        }>
          <div class="everyday-rail-sections">
            <RailSection label="Files" count={files()} icon="file" />
            <RailSection label="Notes" count={notes()} icon="receipt" />
            <RailSection label="Next steps" count={nextSteps()} icon="chevron" />
          </div>
        </Show>
      </aside>
    </Show>
  );
}

function RailSection(props: { label: string; count: number; icon: "file" | "receipt" | "chevron" }) {
  return (
    <button class="everyday-rail-section" type="button" aria-label={`${props.label}, ${props.count}`}>
      <Icon name={props.icon} />
      <span>{props.label}</span>
      <span class="everyday-rail-count">{props.count}</span>
      <Icon name="chevron" />
    </button>
  );
}
