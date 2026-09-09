import { For, Show, createMemo } from "solid-js";
import { activeId, itemsOf, presentationOf, presentationMode, setEverydayRailOpen } from "../store";
import Icon, { type IconName } from "./Icon";

/**
 * The Everyday companion rail provides a stable, clean place for useful context:
 * files, notes, and next steps without crowding the conversation.
 */
export default function EverydayContextRail() {
  const task = createMemo(() => activeId());
  const timeline = createMemo(() => presentationOf(task()));
  const files = createMemo(() => itemsOf(task()).filter((item) => item.kind === "tool").length);
  const notes = createMemo(() => timeline()?.items.length ?? 0);
  const nextSteps = createMemo(() => timeline()?.goal?.additions.length ?? 0);

  return (
    <Show when={presentationMode() === "everyday"}>
      <aside class="everyday-rail" aria-label="Helpful details">
        <div class="everyday-rail-head">
          <h2>Helpful details</h2>
          <button
            class="everyday-rail-close"
            type="button"
            title="Close helpful details"
            aria-label="Close helpful details"
            onClick={() => setEverydayRailOpen(false)}
          >
            <Icon name="close" size={14} />
          </button>
        </div>

        <div class="everyday-rail-body">
          <div class="everyday-rail-empty">
            <div class="everyday-rail-empty-icon">
              <Icon name="file" size={26} />
            </div>
            <p class="everyday-rail-empty-text">Things related to your conversation will appear here.</p>
          </div>

          <div class="everyday-rail-sections">
            <RailSection label="Files" count={files()} icon="file" />
            <RailSection label="Notes" count={notes()} icon="receipt" />
            <RailSection label="Next steps" count={nextSteps()} icon="spark" />
          </div>
        </div>
      </aside>
    </Show>
  );
}

function RailSection(props: { label: string; count: number; icon: IconName }) {
  return (
    <button class="everyday-rail-section" type="button" aria-label={`${props.label}, ${props.count}`}>
      <Icon name={props.icon} size={15} />
      <span>{props.label}</span>
      <span class="everyday-rail-count">{props.count}</span>
      <Icon name="chevron" size={13} />
    </button>
  );
}
