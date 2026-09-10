import type { Accessor } from "solid-js";

const FOCUSABLE =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

function focusableIn(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (element) => element.offsetParent !== null || element === document.activeElement,
  );
}

/** Keep keyboard focus inside an Admin modal and restore it when it closes. */
export function trapFocus(element: HTMLElement, _accessor?: Accessor<unknown>): void {
  const previous = document.activeElement as HTMLElement | null;
  queueMicrotask(() => (focusableIn(element)[0] ?? element).focus({ preventScroll: true }));

  const onKeydown = (event: KeyboardEvent) => {
    if (event.key !== "Tab") return;
    const focusable = focusableIn(element);
    if (focusable.length === 0) {
      event.preventDefault();
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey ? document.activeElement === first || !element.contains(document.activeElement) : document.activeElement === last) {
      event.preventDefault();
      (event.shiftKey ? last : first).focus();
    }
  };
  element.addEventListener("keydown", onKeydown);

  const observer = new MutationObserver(() => {
    if (!document.contains(element)) {
      element.removeEventListener("keydown", onKeydown);
      if (previous && document.contains(previous)) previous.focus({ preventScroll: true });
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
