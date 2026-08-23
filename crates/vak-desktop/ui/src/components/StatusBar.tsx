import { createMemo, For, Show } from "solid-js";
import { activeId, density, health, setDensity, usageOf } from "../store";
import { loadHealth } from "../App";
import * as api from "../api";
import type { JSX } from "solid-js";

function Ring(props: { pct: number; label: string }): JSX.Element {
  const r = 9;
  const c = 2 * Math.PI * r;
  const clamped = () => Math.max(0, Math.min(1, props.pct));
  const color = () => (clamped() > 0.85 ? "#d86f72" : clamped() > 0.6 ? "#d4a85d" : "#df795f");
  return (
    <div class="ring" title={props.label}>
      <svg width="24" height="24" viewBox="0 0 24 24">
        <circle cx="12" cy="12" r={r} fill="none" stroke="#34342f" stroke-width="3" />
        <circle
          cx="12"
          cy="12"
          r={r}
          fill="none"
          stroke={color()}
          stroke-width="3"
          stroke-linecap="round"
          stroke-dasharray={`${clamped() * c} ${c}`}
          transform="rotate(-90 12 12)"
        />
      </svg>
    </div>
  );
}

export default function StatusBar() {
  const h = createMemo(() => health());
  const usage = createMemo(() => usageOf(activeId()));
  const ctxPct = createMemo(() => {
    const w = h()?.context_window ?? 0;
    return w > 0 ? (usage().input_tokens ?? 0) / w : 0;
  });
  const inTok = createMemo(() => (usage().input_tokens ?? 0).toLocaleString());
  const outTok = createMemo(() => (usage().output_tokens ?? 0).toLocaleString());

  const changeMode = async (mode: string) => {
    try {
      await api.setPermissionMode(mode);
      await loadHealth();
    } catch (e) {
      console.error(e);
    }
  };

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
        <div class="density">
          <For each={["summary", "normal", "verbose"] as const}>
            {(d) => (
              <button class="chip sm" classList={{ on: density() === d }} onClick={() => setDensity(d)}>
                {d}
              </button>
            )}
          </For>
        </div>
        <select
          class="st-mode"
          value={h()?.permission_mode ?? ""}
          onChange={(e) => void changeMode(e.currentTarget.value)}
          title="Permission mode — applies to new tool calls immediately"
        >
          <option value="ReadOnly">Read only</option>
          <option value="WorkspaceWrite">Workspace write</option>
          <option value="FullAccess">Full access</option>
        </select>
        <span class="st-tokens" title={`in ${inTok()} / out ${outTok()}`}>
          ↑{inTok()} ↓{outTok()}
        </span>
        <Ring pct={ctxPct()} label={`context: ${(ctxPct() * 100).toFixed(0)}% of ${((h()?.context_window ?? 0) / 1000).toFixed(0)}k`} />
      </div>
    </footer>
  );
}
