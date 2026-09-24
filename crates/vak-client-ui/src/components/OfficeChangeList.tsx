import { For, Show } from "solid-js";
import type { OfficeChange, OfficeChoice, OfficeReview } from "../api";
import Redline from "./OfficeRedline";
import { kept } from "../officeChoices";

// The semantic change list for an Office draft (docs/design/72, P3): what
// changed, where a person would look for it, before and after, and which
// changes to keep. Everything shown comes from the server's worker-computed
// review; nothing is inferred here, and document text is drawn as text,
// never mounted as HTML.

const KIND_LABEL: Record<OfficeChange["kind"], string> = {
  added: "Added",
  removed: "Removed",
  changed: "Changed",
  moved: "Moved",
};

function ChangeItem(props: { change: OfficeChange; showSection?: boolean; onComment?: (anchor: string) => void }) {
  const change = () => props.change;
  return (
    <article class={`office-change office-change-${change().kind}`}>
      <div class="office-change-head">
        <span class="office-change-kind">{KIND_LABEL[change().kind]}</span>
        <Show when={props.showSection && change().section}><span class="office-change-where">{change().section}</span></Show>
        <Show when={change().anchor}><code>{change().anchor}</code></Show>
        <Show when={props.onComment && change().anchor}>
          <button type="button" class="office-change-comment" aria-label={`Comment on ${change().anchor}`} onClick={() => props.onComment?.(change().anchor)}>Comment</button>
        </Show>
      </div>
      <Show when={change().kind === "changed" && change().before !== null && change().before !== undefined}>
        <p class="office-change-before"><span>Before</span><Redline text={change().before ?? ""} /></p>
      </Show>
      <Show when={change().after}>{(after) => (
        <p class="office-change-after"><span>{change().kind === "changed" ? "After" : ""}</span><Redline text={after()} /></p>
      )}</Show>
      <Show when={change().kind === "removed" && change().before}>{(before) => (
        <p class="office-change-before"><Redline text={before()} /></p>
      )}</Show>
    </article>
  );
}

function Sections(props: { changes: OfficeChange[]; onComment?: (anchor: string) => void }) {
  const sections = () => {
    const order: string[] = [];
    const grouped = new Map<string, OfficeChange[]>();
    for (const change of props.changes) {
      if (!grouped.has(change.section)) {
        order.push(change.section);
        grouped.set(change.section, []);
      }
      grouped.get(change.section)!.push(change);
    }
    return order.map((section) => ({ section, changes: grouped.get(section)! }));
  };
  return (
    <For each={sections()}>{(group) => (
      <section class="office-change-section">
        <h4>{group.section}</h4>
        <For each={group.changes}>{(change) => <ChangeItem change={change} onComment={props.onComment} />}</For>
      </section>
    )}</For>
  );
}

function Choices(props: {
  choices: OfficeChoice[];
  excluded: ReadonlySet<string>;
  onToggle: (id: string) => void;
  onMakeVersion: () => void;
  busy: boolean;
  error: string | null;
  onComment?: (anchor: string) => void;
}) {
  const labelOf = (id: string) => props.choices.find((choice) => choice.id === id)?.label ?? id;
  const keeping = () => kept(props.choices, props.excluded).length;
  return (
    <div class="office-choices">
      <p class="office-choice-hint">Every change is kept. Uncheck one to leave it out; a change that builds on it goes with it.</p>
      <For each={props.choices}>{(choice) => {
        const inputId = `office-choice-${choice.id.replace(/[^A-Za-z0-9_-]/g, "-")}`;
        return (
          <article class="office-choice" classList={{ "office-choice-left-out": props.excluded.has(choice.id) }}>
            <div class="office-choice-head">
              <input id={inputId} type="checkbox" checked={!props.excluded.has(choice.id)} onChange={() => props.onToggle(choice.id)} />
              <label for={inputId}>{choice.label}</label>
            </div>
            <Show when={choice.requires.length > 0}>
              <p class="office-choice-requires">Builds on: {choice.requires.map(labelOf).join(", ")}</p>
            </Show>
            <Show when={choice.changes.length > 0} fallback={<p class="office-change-empty">No visible change.</p>}>
              <For each={choice.changes}>{(change) => <ChangeItem change={change} showSection onComment={props.onComment} />}</For>
            </Show>
          </article>
        );
      }}</For>
      <Show when={props.excluded.size > 0}>
        <div class="office-choice-footer" role="status">
          <span>
            {keeping() === 0
              ? "Nothing is kept. To take none of this draft, close Review without accepting."
              : `Keeping ${keeping()} of ${props.choices.length} changes.`}
          </span>
          <button type="button" class="btn primary sm" disabled={props.busy || keeping() === 0} onClick={() => props.onMakeVersion()}>
            {props.busy ? "Making version…" : `Make a version with ${keeping() === 1 ? "this change" : `these ${keeping()} changes`}`}
          </button>
        </div>
      </Show>
      <Show when={props.error}>{(message) => <p role="alert" class="inline-error">{message()}</p>}</Show>
    </div>
  );
}

export default function OfficeChangeList(props: {
  review: OfficeReview;
  excluded?: ReadonlySet<string>;
  onToggle?: (id: string) => void;
  onMakeVersion?: () => void;
  busy?: boolean;
  error?: string | null;
  onOpenVersion?: (candidateId: string) => void;
  /** Point the review comment at a change's anchor (F9). */
  onComment?: (anchor: string) => void;
}) {
  const choosable = () => (props.review.choices?.length ?? 0) >= 2 && !!props.onToggle && !!props.onMakeVersion;
  return (
    <div class="office-change-list" aria-label="Changes in this draft">
      <p class="office-change-compared">
        {props.review.compared_with === "workspace" ? "Compared with the current workspace file." : "A new file; nothing to compare with."}
      </p>
      <Show when={(props.review.impact?.length ?? 0) > 0}>
        <ul class="office-impact" aria-label="What accepting also does">
          <For each={props.review.impact}>{(impact) => (
            <li classList={{ "office-impact-warning": impact.warning }}>
              <strong>{impact.kind === "signature" ? "Signature" : "Sensitivity label"}</strong>
              <span>{impact.message}</span>
            </li>
          )}</For>
        </ul>
      </Show>
      <Show when={props.review.flags.length > 0}>
        <p class="office-change-flags" role="note">{props.review.flags.join(" · ")}</p>
      </Show>
      <Show when={props.review.narrowed_from}>{(from) => (
        <div class="office-choice-note" role="note">
          <span>This version keeps {from().keep.length} of the full draft's changes.</span>
          <Show when={props.onOpenVersion}>
            <button type="button" class="btn sm" onClick={() => props.onOpenVersion?.(from().candidate_id)}>Choose again from the full draft</button>
          </Show>
        </div>
      )}</Show>
      <Show when={!props.review.narrowed_from && !choosable() && props.review.choices_unavailable && props.review.changes.length > 1}>
        <p class="office-choice-note" role="note">This draft is accepted or rejected whole: {props.review.choices_unavailable}</p>
      </Show>
      <Show when={props.review.changes.length > 0} fallback={<p class="office-change-empty">No visible change.</p>}>
        <Show when={choosable()} fallback={<Sections changes={props.review.changes} onComment={props.onComment} />}>
          <Choices
            choices={props.review.choices ?? []}
            excluded={props.excluded ?? new Set()}
            onToggle={(id) => props.onToggle?.(id)}
            onMakeVersion={() => props.onMakeVersion?.()}
            busy={props.busy ?? false}
            error={props.error ?? null}
            onComment={props.onComment}
          />
        </Show>
      </Show>
    </div>
  );
}
