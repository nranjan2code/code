import { createMemo, Show } from "solid-js";
import { health } from "../store";

export default function StatusBar() {
  const h = createMemo(() => health());

  return (
    <footer class="statusbar">
      <div class="st-left">
        <span class="st-item st-model" classList={{ offline: !h() }} title={`provider: ${h()?.provider ?? "connecting"}`}>
          {h()?.model ?? "Connecting…"}
        </span>
        <span class="st-item st-sandbox" title="sandbox backend">{h()?.sandbox}</span>
        <Show when={(h()?.warnings?.length ?? 0) > 0}>
          <span class="st-item warn" title={JSON.stringify(h()?.warnings)}>
            ⚠ {h()?.warnings?.length} warning(s)
          </span>
        </Show>
      </div>
      <div class="st-right">
        <Show when={h()?.provider}>
          <span class="st-item" title="active provider">{h()?.provider}</span>
        </Show>
      </div>
    </footer>
  );
}
