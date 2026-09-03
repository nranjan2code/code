import { dockWidth, setDockWidth, setSidebarWidth, sidebarWidth } from "../store";

export default function ResizeHandle(props: { side: "sidebar" | "dock" }) {
  const value = () => props.side === "sidebar" ? sidebarWidth() : dockWidth();
  const update = (width: number) => {
    if (props.side === "sidebar") {
      const next = Math.max(220, Math.min(360, width));
      setSidebarWidth(next);
      localStorage.setItem("vak.sidebarWidth", String(next));
    } else {
      const next = Math.max(340, Math.min(window.innerWidth * 0.62, width));
      setDockWidth(next);
      localStorage.setItem("vak.dockWidth", String(next));
    }
  };

  const begin = (event: PointerEvent) => {
    event.preventDefault();
    const startX = event.clientX;
    const initial = props.side === "sidebar" ? sidebarWidth() : dockWidth();
    document.body.classList.add("is-resizing");

    const move = (next: PointerEvent) => {
      const delta = next.clientX - startX;
      update(initial + (props.side === "sidebar" ? delta : -delta));
    };
    const end = () => {
      document.body.classList.remove("is-resizing");
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end, { once: true });
  };

  return (
    <div
      class={`resize-handle ${props.side}-resizer`}
      role="separator"
      tabIndex={0}
      aria-label={`Resize ${props.side}`}
      aria-orientation="vertical"
      aria-valuemin={props.side === "sidebar" ? 220 : 340}
      aria-valuemax={props.side === "sidebar" ? 360 : Math.round(window.innerWidth * 0.62)}
      aria-valuenow={Math.round(value())}
      onPointerDown={begin}
      onDblClick={() => update(props.side === "sidebar" ? 278 : 520)}
      onKeyDown={(event) => {
        if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
        event.preventDefault();
        const direction = event.key === "ArrowRight" ? 1 : -1;
        update(value() + direction * (props.side === "sidebar" ? 10 : -10));
      }}
    />
  );
}
