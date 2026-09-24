import { For, Show } from "solid-js";
import type { OfficeChange, OfficeDiff } from "../api";
import { parseRedline } from "../officeRedline";

// The semantic change list for an Office draft (docs/design/72, P3): what
// changed, where a person would look for it, before and after. Everything
// shown comes from the server's worker-computed diff; nothing is inferred
// here, and document text is drawn as text, never mounted as HTML.

const KIND_LABEL: Record<OfficeChange["kind"], string> = {
  added: "Added",
  removed: "Removed",
  changed: "Changed",
  moved: "Moved",
};

function Redline(props: { text: string }) {
  return (
    <For each={parseRedline(props.text)}>{(segment) => {
      switch (segment.kind) {
        case "inserted":
          return <ins class="office-redline-inserted" title={`Inserted by ${segment.author}`}>{segment.text}</ins>;
        case "deleted":
          return <del class="office-redline-deleted" title={`Deleted by ${segment.author}`}>{segment.text}</del>;
        case "hidden":
          return <span class="office-redline-hidden" title="Hidden text in the document">{segment.text}</span>;
        case "white":
          return <span class="office-redline-hidden" title="White text in the document">{segment.text}</span>;
        default:
          return <>{segment.text}</>;
      }
    }}</For>
  );
}

export default function OfficeChangeList(props: { diff: OfficeDiff }) {
  const sections = () => {
    const order: string[] = [];
    const grouped = new Map<string, OfficeChange[]>();
    for (const change of props.diff.changes) {
      if (!grouped.has(change.section)) {
        order.push(change.section);
        grouped.set(change.section, []);
      }
      grouped.get(change.section)!.push(change);
    }
    return order.map((section) => ({ section, changes: grouped.get(section)! }));
  };
  return (
    <div class="office-change-list" aria-label="Changes in this draft">
      <p class="office-change-compared">
        {props.diff.compared_with === "workspace" ? "Compared with the current workspace file." : "A new file; nothing to compare with."}
      </p>
      <Show when={props.diff.flags.length > 0}>
        <p class="office-change-flags" role="note">{props.diff.flags.join(" · ")}</p>
      </Show>
      <Show when={props.diff.changes.length > 0} fallback={<p class="office-change-empty">No visible change.</p>}>
        <For each={sections()}>{(group) => (
          <section class="office-change-section">
            <h4>{group.section}</h4>
            <For each={group.changes}>{(change) => (
              <article class={`office-change office-change-${change.kind}`}>
                <div class="office-change-head">
                  <span class="office-change-kind">{KIND_LABEL[change.kind]}</span>
                  <Show when={change.anchor}><code>{change.anchor}</code></Show>
                </div>
                <Show when={change.kind === "changed" && change.before !== null && change.before !== undefined}>
                  <p class="office-change-before"><span>Before</span><Redline text={change.before ?? ""} /></p>
                </Show>
                <Show when={change.after}>{(after) => (
                  <p class="office-change-after"><span>{change.kind === "changed" ? "After" : ""}</span><Redline text={after()} /></p>
                )}</Show>
                <Show when={change.kind === "removed" && change.before}>{(before) => (
                  <p class="office-change-before"><Redline text={before()} /></p>
                )}</Show>
              </article>
            )}</For>
          </section>
        )}</For>
      </Show>
    </div>
  );
}
