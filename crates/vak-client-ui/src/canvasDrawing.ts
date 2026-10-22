export interface CanvasViewport {
  width: number;
  height: number;
  pixelRatio: number;
  pixelWidth: number;
  pixelHeight: number;
}

/** CSS size and physical backing pixels are separate, including fractional display scale. */
export function canvasViewport(width: number, height: number, pixelRatio: number): CanvasViewport {
  const ratio = Number.isFinite(pixelRatio) && pixelRatio > 0 ? pixelRatio : 1;
  const w = Number.isFinite(width) ? Math.max(0, width) : 0;
  const h = Number.isFinite(height) ? Math.max(0, height) : 0;
  return { width: w, height: h, pixelRatio: ratio, pixelWidth: Math.round(w * ratio), pixelHeight: Math.round(h * ratio) };
}

/** One coalesced viewport update per paint, including moving between different-DPI displays. */
export function observeCanvasViewport(element: HTMLElement, change: (viewport: CanvasViewport) => void) {
  let frame = 0;
  let disposed = false;
  let query: MediaQueryList;
  let size: { width: number; height: number } | undefined;
  const measure = () => {
    frame = 0;
    if (disposed) return;
    const measured = size ?? element.getBoundingClientRect();
    change(canvasViewport(measured.width, measured.height, window.devicePixelRatio));
  };
  const schedule = () => { if (!disposed && !frame) frame = requestAnimationFrame(measure); };
  const densityChanged = () => { watchDensity(); schedule(); };
  const watchDensity = () => {
    query?.removeEventListener("change", densityChanged);
    query = matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
    query.addEventListener("change", densityChanged);
  };
  const observer = new ResizeObserver(([entry]) => { size = entry.contentRect; schedule(); });
  observer.observe(element);
  watchDensity();
  schedule();
  return () => { disposed = true; observer.disconnect(); query.removeEventListener("change", densityChanged); cancelAnimationFrame(frame); };
}

export interface CanvasFrame extends CanvasViewport { time: number; delta: number }

/** Owned renderers share sizing and scheduling; the renderer owns its drawing/GPU context. */
export function createCanvasDrawingSurface(
  canvas: HTMLCanvasElement,
  draw: (frame: CanvasFrame) => void,
  options: { continuous?: boolean | (() => boolean); onError: (error: unknown) => void; onFrame?: (timing: { interval: number; paint: number }) => void },
) {
  let viewport = canvasViewport(0, 0, window.devicePixelRatio);
  let frame = 0;
  let last = 0;
  let disposed = false;
  let intersecting = true;
  let contextLost = false;
  let failed = false;
  const visible = () => !disposed && !failed && !contextLost && intersecting && document.visibilityState === "visible" && viewport.pixelWidth > 0 && viewport.pixelHeight > 0;
  const paint = (time: number) => {
    frame = 0;
    if (!visible()) { last = 0; return; }
    const delta = last ? time - last : 0;
    const start = performance.now();
    try {
      if (canvas.width !== viewport.pixelWidth) canvas.width = viewport.pixelWidth;
      if (canvas.height !== viewport.pixelHeight) canvas.height = viewport.pixelHeight;
      draw({ ...viewport, time, delta });
      options.onFrame?.({ interval: delta, paint: performance.now() - start });
    } catch (error) {
      failed = true;
      options.onError(error);
      return;
    }
    last = time;
    if (typeof options.continuous === "function" ? options.continuous() : options.continuous) invalidate();
  };
  const invalidate = () => { if (visible() && !frame) frame = requestAnimationFrame(paint); };
  const resume = () => {
    last = 0;
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    invalidate();
  };
  const stopViewport = observeCanvasViewport(canvas, value => { viewport = value; invalidate(); });
  const observer = new IntersectionObserver(entries => { intersecting = entries.some(entry => entry.isIntersecting); resume(); });
  observer.observe(canvas);
  const lost = (event: Event) => { event.preventDefault(); contextLost = true; resume(); };
  const restored = () => { contextLost = false; resume(); };
  canvas.addEventListener("contextlost", lost);
  canvas.addEventListener("webglcontextlost", lost);
  canvas.addEventListener("contextrestored", restored);
  canvas.addEventListener("webglcontextrestored", restored);
  document.addEventListener("visibilitychange", resume);
  return {
    invalidate,
    viewport: () => viewport,
    dispose() {
      disposed = true;
      cancelAnimationFrame(frame);
      stopViewport(); observer.disconnect();
      canvas.removeEventListener("contextlost", lost); canvas.removeEventListener("webglcontextlost", lost);
      canvas.removeEventListener("contextrestored", restored); canvas.removeEventListener("webglcontextrestored", restored);
      document.removeEventListener("visibilitychange", resume);
    },
  };
}
