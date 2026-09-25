import { createUniqueId, onCleanup, onMount, Show, type JSX } from "solid-js";
import { trapFocus } from "../focusTrap";
import Icon from "./Icon";

void trapFocus;

// Open sheets, newest last. Escape belongs to the newest one wherever focus
// happens to be, so it can never fall through to the page behind.
const openSheets: symbol[] = [];

/**
 * The one dialog frame (docs/design/75 §6.4): a title, the close button at
 * the top right, focus kept inside, Escape to close and focus returned to
 * what opened it. Escape is handled here, for the newest sheet, and goes no
 * further, so it can never reach the app-wide shortcut that stops a run.
 */
export default function Sheet(props: {
  title: JSX.Element;
  subtitle?: JSX.Element;
  onClose: () => void;
  /** `wide` for sheets with a document or table; `narrow` for a question. */
  size?: "narrow" | "normal" | "wide";
  class?: string;
  /** Pinned under the scrolling body: the sheet's decision buttons. */
  footer?: JSX.Element;
  /** Stops Escape, the backdrop and the close button while work is saving. */
  busy?: boolean;
  children: JSX.Element;
}) {
  const titleId = createUniqueId();
  const close = () => { if (!props.busy) props.onClose(); };
  const self = Symbol("sheet");
  let root!: HTMLElement;
  onMount(() => {
    openSheets.push(self);
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || openSheets.at(-1) !== self) return;
      event.stopPropagation();
      event.preventDefault();
      close();
    };
    // Capture on window runs before every other Escape handler in the app.
    window.addEventListener("keydown", onKey, true);
    // A sheet can mount while something else is still taking focus (Review
    // opens as the details panel does); once it has drawn, focus goes inside.
    requestAnimationFrame(() => requestAnimationFrame(() => {
      if (root.contains(document.activeElement)) return;
      (root.querySelector<HTMLElement>("input, textarea, select, button:not(.sheet-close)") ?? root).focus({ preventScroll: true });
    }));
    onCleanup(() => {
      window.removeEventListener("keydown", onKey, true);
      openSheets.splice(openSheets.indexOf(self), 1);
    });
  });
  return (
    <div class="modal-back" role="presentation" onClick={(event) => { if (event.target === event.currentTarget) close(); }}>
      <section
        class={`modal sheet sheet-${props.size ?? "normal"}${props.class ? ` ${props.class}` : ""}`}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        ref={root}
        use:trapFocus
      >
        <header class="sheet-header">
          <div>
            <h2 id={titleId}>{props.title}</h2>
            <Show when={props.subtitle}><p>{props.subtitle}</p></Show>
          </div>
          <button type="button" class="icon-button subtle sheet-close" aria-label="Close" disabled={props.busy} onClick={close}><Icon name="close" /></button>
        </header>
        <div class="sheet-body">{props.children}</div>
        <Show when={props.footer}><footer class="sheet-footer">{props.footer}</footer></Show>
      </section>
    </div>
  );
}
