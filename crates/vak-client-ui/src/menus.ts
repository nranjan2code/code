/**
 * Dropdowns are `<details data-menu>`. A browser closes one only when its
 * summary is pressed again, so Escape and a press anywhere outside it close
 * it here. Escape closes an open menu before it does anything else: the
 * same key stops a run when nothing is open.
 */
export function closeOpenMenus(outside?: EventTarget | null): boolean {
  let closed = false;
  for (const menu of document.querySelectorAll<HTMLDetailsElement>("details[data-menu][open]")) {
    if (outside instanceof Node && menu.contains(outside)) continue;
    menu.removeAttribute("open");
    closed = true;
  }
  return closed;
}

export function dismissMenusOnPressOutside(): () => void {
  const press = (event: PointerEvent) => void closeOpenMenus(event.target);
  document.addEventListener("pointerdown", press, true);
  return () => document.removeEventListener("pointerdown", press, true);
}
