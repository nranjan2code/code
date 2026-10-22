import { createSignal, onCleanup } from "solid-js";
import { setCanvasSplitWidth } from "../../store";

/** Resizing is frame-coalesced; pointer capture keeps previews from swallowing drags. */
export default function CanvasResize() {
  const [width, setWidth] = createSignal(0);
  let root: HTMLDivElement | undefined;
  let frame = 0;
  let wanted = 0;
  let cleanup: (() => void) | undefined;
  const apply = (next: number) => {
    const value = Math.max(440, Math.min(window.innerWidth - 360, next));
    setWidth(value);
    setCanvasSplitWidth(value);
  };
  onCleanup(() => { cleanup?.(); cancelAnimationFrame(frame); });
  return <div ref={root} class="canvas-resizer" role="separator" aria-label="Resize Canvas"
    aria-orientation="vertical" tabIndex={0} aria-valuemin={440} aria-valuemax={Math.max(440, window.innerWidth - 360)}
    aria-valuenow={Math.round(width() || window.innerWidth * 0.65)}
    onDblClick={() => { setCanvasSplitWidth(null); setWidth(0); }}
    onKeyDown={event => {
      if (!["ArrowLeft", "ArrowRight", "Home"].includes(event.key)) return;
      event.preventDefault();
      if (event.key === "Home") { setCanvasSplitWidth(null); setWidth(0); return; }
      const measured = root?.parentElement?.getBoundingClientRect().width ?? window.innerWidth * 0.65;
      apply(measured + (event.key === "ArrowLeft" ? 16 : -16));
    }}
    onPointerDown={event => {
      if (event.button !== 0 || !root) return;
      event.preventDefault();
      const element = root;
      const start = event.clientX;
      const initial = element.parentElement?.getBoundingClientRect().width ?? window.innerWidth * 0.65;
      cleanup?.();
      element.setPointerCapture(event.pointerId);
      document.body.classList.add("is-resizing");
      const move = (next: PointerEvent) => {
        wanted = initial + start - next.clientX;
        if (!frame) frame = requestAnimationFrame(() => { frame = 0; apply(wanted); });
      };
      const end = () => {
        if (frame) { cancelAnimationFrame(frame); frame = 0; apply(wanted); }
        document.body.classList.remove("is-resizing");
        element.removeEventListener("pointermove", move);
        element.removeEventListener("pointerup", end);
        element.removeEventListener("pointercancel", end);
        element.removeEventListener("lostpointercapture", end);
        cleanup = undefined;
        if (element.hasPointerCapture(event.pointerId)) element.releasePointerCapture(event.pointerId);
      };
      cleanup = end;
      element.addEventListener("pointermove", move);
      element.addEventListener("pointerup", end);
      element.addEventListener("pointercancel", end);
      element.addEventListener("lostpointercapture", end);
    }} />;
}
