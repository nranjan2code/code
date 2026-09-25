// Solid custom directive: `use:trapFocus` on a dialog's root element.
//
// None of the ten `role="dialog"` modals in this app trapped focus, set
// initial focus, or restored focus on close — Tab left the dialog into the
// page behind it, and the element that had focus before the modal opened
// was simply forgotten. This is the one place that behavior is
// implemented, so every modal picks it up with one attribute instead of
// reimplementing (or continuing to skip) it individually.
//
// Usage: `<div role="dialog" aria-modal="true" use:trapFocus>`.
// Import the module once per file (`import "../focusTrap"` or the named
// export below) so the TS compiler keeps the JSX directive registered.

import type { Accessor } from "solid-js";

const FOCUSABLE =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

function focusableIn(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (el) => el.offsetParent !== null || el === document.activeElement,
  );
}

/** A control's name, to find it again if it is re-rendered while the dialog
 * is open (a result card redraws when its review closes). */
function controlName(el: HTMLElement | null): string | null {
  if (!el || el === document.body) return null;
  const name = el.getAttribute("aria-label") ?? el.innerText?.trim();
  return name ? `${el.tagName}:${name}` : null;
}

function sameControl(name: string | null): HTMLElement | null {
  if (!name) return null;
  const tag = name.slice(0, name.indexOf(":"));
  return Array.from(document.querySelectorAll<HTMLElement>(tag))
    .filter((el) => el.offsetParent !== null && controlName(el) === name)
    .at(-1) ?? null;
}

export function trapFocus(el: HTMLElement, _accessor?: Accessor<unknown>): void {
  const previouslyFocused = document.activeElement as HTMLElement | null;
  const openerName = controlName(previouslyFocused);
  const hadTabIndex = el.hasAttribute("tabindex");
  if (!hadTabIndex) el.setAttribute("tabindex", "-1");

  // Initial focus: the first focusable element, or the dialog itself (it
  // needs `tabindex="-1"` in markup were it not already interactive) so a
  // dialog with no controls yet (a loading state) still receives focus
  // rather than leaving it on whatever was behind it.
  queueMicrotask(() => {
    const first = focusableIn(el)[0];
    (first ?? el).focus({ preventScroll: true });
  });

  const onKeydown = (e: KeyboardEvent) => {
    if (e.key !== "Tab") return;
    const focusable = focusableIn(el);
    if (focusable.length === 0) {
      e.preventDefault();
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement;
    if (e.shiftKey) {
      if (active === first || !el.contains(active)) {
        e.preventDefault();
        last.focus();
      }
    } else {
      if (active === last || !el.contains(active)) {
        e.preventDefault();
        first.focus();
      }
    }
  };
  el.addEventListener("keydown", onKeydown);

  // Solid calls a directive's cleanup by returning nothing and relying on
  // `onCleanup` — but directives run outside a component's own reactive
  // root when attached this way, so this hooks the element's own
  // disconnection instead, which fires reliably when Solid removes the
  // dialog from the DOM (the only way any of these modals close).
  const observer = new MutationObserver(() => {
    if (!document.contains(el)) {
      el.removeEventListener("keydown", onKeydown);
      if (!hadTabIndex) el.removeAttribute("tabindex");
      // Restore focus to whatever opened the dialog — a closed modal must
      // not strand focus on `document.body`.
      if (previouslyFocused && document.contains(previouslyFocused)) previouslyFocused.focus({ preventScroll: true });
      else {
        // The replacement can take a few frames to draw; give up after about
        // a second, or as soon as anything else takes focus.
        let tries = 60;
        const retry = () => {
          if (document.activeElement !== document.body) return;
          const same = sameControl(openerName);
          if (same) same.focus({ preventScroll: true });
          else if (--tries > 0) requestAnimationFrame(retry);
        };
        requestAnimationFrame(retry);
      }
      observer.disconnect();
    }
  });
  observer.observe(document.body, { childList: true, subtree: true });
}

declare module "solid-js" {
  namespace JSX {
    interface Directives {
      trapFocus: boolean | undefined;
    }
  }
}
