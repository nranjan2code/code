import { For, Show, createMemo } from "solid-js";
import { activeId, hydratingId, presentationErrorOf, presentationOf, presentationMode, setDockTab, setEverydayRailOpen, workbenchExecutions } from "../store";
import Icon, { type IconName } from "./Icon";

/**
 * The Everyday companion rail provides a stable, clean place for useful context:
 * files, notes, and next steps without crowding the conversation.
 */
export default function EverydayContextRail(props: { onRetry?: (id: string) => void }) {
  const task = createMemo(() => activeId());
  const timeline = createMemo(() => presentationOf(task()));
  const presentationError = createMemo(() => presentationErrorOf(task()));
  const loading = createMemo(() => !!task() && hydratingId() === task() && !timeline());
  // Only count artifacts that the workbench can actually open. Transcript
  // messages and tool calls are not files, and counting them made this rail
  // claim files existed when there were none.
  const files = createMemo(() => workbenchExecutions()
    .filter((execution) => !execution.ownerSessionId || execution.ownerSessionId === task())
    .reduce((count, execution) => count + execution.artifacts.length, 0));
  const notes = createMemo(() => timeline()?.items.filter((item) => item.kind === "outcome").length ?? 0);
  const nextSteps = createMemo(() => timeline()?.goal?.additions.length ?? 0);

  return (
    <Show when={presentationMode() === "everyday"}>
      <aside class="everyday-rail" aria-label="Helpful details" data-testid="everyday-context-rail">
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
          <Show when={loading()}>
            <div class="everyday-rail-empty" role="status" aria-live="polite">
              <div class="everyday-rail-empty-icon"><Icon name="spark" size={26} /></div>
              <p class="everyday-rail-empty-text">Loading conversation details…</p>
            </div>
          </Show>
          <Show when={!loading() && presentationError()}>
            <div class="everyday-rail-empty" role="status">
              <div class="everyday-rail-empty-icon"><Icon name="warning" size={26} /></div>
              <p class="everyday-rail-empty-text">Conversation details are temporarily unavailable.</p>
              <button type="button" class="everyday-rail-retry" onClick={() => task() && props.onRetry?.(task()!)}>
                Retry details
              </button>
            </div>
          </Show>
          <Show when={!loading() && !presentationError() && (timeline()?.items.length ?? 0) === 0} fallback={
            <div class="everyday-rail-preview" aria-label="Conversation highlights">
              <For each={timeline()?.items.slice(0, 4) ?? []}>
                {(item) => <div class="everyday-rail-preview-row">
                  <span class="dot" classList={{ run: item.status === "running" }} />
                  <span>{item.fallback_text || (item.kind === "outcome" ? "Result" : "Conversation update")}</span>
                </div>}
              </For>
            </div>
          }>
            <div class="everyday-rail-empty">
              <div class="everyday-rail-empty-icon">
                <Icon name="file" size={26} />
              </div>
              <p class="everyday-rail-empty-text">Things related to your conversation will appear here.</p>
            </div>
          </Show>

          <div class="everyday-rail-sections">
            <RailSection label="Results" count={files()} icon="preview" tab="workbench" />
            <RailSection label="What happened" count={notes()} icon="receipt" tab="workbench" />
            <RailSection label="Next steps" count={nextSteps()} icon="spark" tab="commitments" />
          </div>
        </div>
      </aside>
    </Show>
  );
}

function RailSection(props: { label: string; count: number; icon: IconName; tab: "editor" | "workbench" | "commitments" }) {
  return (
    <button class="everyday-rail-section" type="button" aria-label={`Open ${props.label}, ${props.count}`} onClick={() => setDockTab(props.tab)}>
      <Icon name={props.icon} size={15} />
      <span>{props.label}</span>
      <span class="everyday-rail-count">{props.count}</span>
      <Icon name="chevron" size={13} />
    </button>
  );
}
