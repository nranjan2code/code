import type { JSX } from "solid-js";
import { canvasDevice } from "../../store";

/** Frames a page at desktop, tablet or phone width. */
export default function DeviceViewport(props: { children: JSX.Element }) {
  const device = canvasDevice;
  return (
    <div
      class="artifact-canvas-viewport"
      classList={{
        "device-desktop": device() === "desktop",
        "device-tablet": device() === "tablet",
        "device-mobile": device() === "mobile",
      }}
    >
      <div class="artifact-canvas-frame-container">{props.children}</div>
    </div>
  );
}
