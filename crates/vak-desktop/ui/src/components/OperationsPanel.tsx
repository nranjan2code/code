import { createEffect, createSignal, For, Show } from "solid-js";
import * as api from "../api";
import Icon from "./Icon";

function Status(props: { value: string; good?: boolean }) {
  return <span class="settings-status" classList={{ good: props.good ?? ["running", "ok", "enabled"].includes(props.value), bad: !(props.good ?? ["running", "ok", "enabled"].includes(props.value)) }}>{props.value}</span>;
}

export default function OperationsPanel(props: { onNotice?: (text: string) => void }) {
  const [data, setData] = createSignal<api.OpsDiagnostics | null>(null);
  const [doctor, setDoctor] = createSignal<api.DoctorReport | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const refresh = async () => {
    setLoading(true);
    try {
      const [next, doc] = await Promise.all([api.opsDiagnostics(), api.doctor()]);
      setData(next); setDoctor(doc); setError(null);
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setLoading(false); }
  };
  createEffect(() => { void refresh(); const timer = setInterval(() => void refresh(), 8000); return () => clearInterval(timer); });
  const action = async (service: "gateway", verb: "start" | "stop" | "restart") => {
    try {
      const result = await api.opsAction(service, verb);
      if (!result.ok) throw new Error(result.error ?? "service action failed");
      await refresh();
    }
    catch (e) { props.onNotice?.(e instanceof Error ? e.message : String(e)); }
  };
  return <div class="operations-panel">
    <Show when={error()}><div class="settings-warning"><Icon name="shield" /> {error()} <button class="settings-button" onClick={() => void refresh()}>Retry</button></div></Show>
    <Show when={loading() && !data()}><div class="operations-empty">Loading operational status…</div></Show>
    <Show when={data()}>
      <section class="operations-hero"><div><h2>Operations</h2><p>Live Runtime health and managed service status.</p></div><button class="settings-button" onClick={() => void refresh()}>Refresh</button></section>
      <div class="operations-grid">
        <section class="operation-card"><header><div><h3>Runtime health</h3><p>Current execution environment</p></div><Status value={data()!.health.status} /></header>
          <Show when={data()!.health.warnings.length}><div class="settings-warning">{data()!.health.warnings.length} configuration warning(s) need attention.</div></Show>
        </section>
        <section class="operation-card"><header><div><h3>Background services</h3><p>Managed by vak-ops</p></div><Status value={data()!.services.gateway_healthy ? "healthy" : "unreachable"} /></header>
          <div class="operation-service"><div><strong>Gateway</strong><small>{data()!.services.gateway.state}</small></div><div><button class="settings-button" onClick={() => void action("gateway", "restart")}>Restart</button><button class="settings-button" onClick={() => void action("gateway", data()!.services.gateway.state === "running" ? "stop" : "start")}>{data()!.services.gateway.state === "running" ? "Stop" : "Start"}</button></div></div>
        </section>
        <section class="operation-card doctor-card">
          <header><div><h3>Diagnostics</h3><p>Runtime health checks and facts</p></div>
            <Status value={doctor() ? (doctor()!.failures === 0 ? "ok" : `${doctor()!.failures} failing`) : "unknown"} />
          </header>
          <Show when={doctor()}>
            <div class="doctor-checks">
              <For each={doctor()!.checks}>
                {(check) => (
                  <div class="doctor-check" title={check.detail}>
                    <span class="dot" classList={{ ok: check.ok, fail: !check.ok }} aria-hidden="true" />
                    <span class="doctor-check-label">{check.label}</span>
                    <span class="doctor-check-detail">{check.detail}</span>
                  </div>
                )}
              </For>
            </div>
            <div class="operation-facts doctor-facts"><For each={doctor()!.facts}>{(fact) => <span title={fact}>{fact}</span>}</For></div>
          </Show>
        </section>
      </div>
    </Show>
  </div>;
}
