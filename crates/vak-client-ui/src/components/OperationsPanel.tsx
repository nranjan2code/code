import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import * as api from "../api";
import { host } from "../host";
import { activeAgentId } from "../store";
import Icon from "./Icon";
import Skeleton from "./Skeleton";

function Status(props: { value: string; good?: boolean }) {
  return <span class="settings-status" classList={{ good: props.good ?? ["running", "ok", "enabled"].includes(props.value), bad: !(props.good ?? ["running", "ok", "enabled"].includes(props.value)) }}>{props.value}</span>;
}

export default function OperationsPanel(props: { onNotice?: (text: string) => void }) {
  const [data, setData] = createSignal<api.OpsDiagnostics | null>(null);
  const [finops, setFinops] = createSignal<api.FinopsStatus | null>(null);
  const [doctor, setDoctor] = createSignal<api.DoctorReport | null>(null);
  const [autostart, setAutostart] = createSignal<boolean>(true);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const refresh = async () => {
    setLoading(true);
    try {
      const [next, spend, doc] = await Promise.all([api.opsDiagnostics(), api.finopsStatus(activeAgentId()), api.doctor()]);
      setData(next); setFinops(spend); setDoctor(doc); setError(null);
      if (host.can("tray") && host.getAutostart) {
        const auto = await host.getAutostart();
        setAutostart(auto);
      }
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setLoading(false); }
  };
  createEffect(() => { void refresh(); const timer = setInterval(() => void refresh(), 8000); onCleanup(() => clearInterval(timer)); });
  const action = async (service: "gateway" | "bridges", verb: "start" | "stop" | "restart") => {
    try { await api.opsAction(service, verb); await refresh(); }
    catch (e) { props.onNotice?.(e instanceof Error ? e.message : String(e)); }
  };
  const toggleAutostart = async () => {
    if (!host.setAutostart) return;
    const next = !autostart();
    try {
      await host.setAutostart(next);
      setAutostart(next);
    } catch (e) {
      props.onNotice?.(e instanceof Error ? e.message : String(e));
    }
  };
  return <div class="operations-panel">
    <Show when={error()}><div class="settings-warning"><Icon name="shield" /> {error()} <button class="settings-button" onClick={() => void refresh()}>Retry</button></div></Show>
    <Show when={loading() && !data()}><Skeleton kind="rows" label="Loading system health" /></Show>
    <Show when={data()}>
      <section class="operations-hero"><div><h2>Services</h2><p>Live health, background services, gateway bindings, flows, and spend controls.</p></div><button class="settings-button" onClick={() => void refresh()}>Refresh</button></section>
      <div class="operations-grid">
        <section class="operation-card"><header><div><h3>Runtime health</h3><p>Where Vakyartha runs</p></div><Status value={data()!.health.status} /></header>
          <div class="operation-facts"><span><b>Provider</b>{data()!.health.provider || "—"}</span><span><b>Model</b>{data()!.health.model || "—"}</span><span><b>Isolation</b>{data()!.health.sandbox || "—"}</span><span><b>Permission</b>{data()!.health.permission_mode || "—"}</span></div>
          <Show when={data()!.health.warnings.length}><div class="settings-warning">{data()!.health.warnings.length} configuration warning(s) need attention.</div></Show>
        </section>
        <section class="operation-card"><header><div><h3>Background services</h3><p>Managed by vak-ops</p></div><Status value={data()!.services.gateway_healthy ? "healthy" : "unreachable"} /></header>
          <For each={["gateway", "bridges"] as const}>{(service) => <div class="operation-service"><div><strong>{service === "gateway" ? "Gateway" : "Chat bridges"}</strong><small>{data()!.services[service].state}</small></div><div><button class="settings-button" onClick={() => void action(service, "restart")}>Restart</button><button class="settings-button" onClick={() => void action(service, data()!.services[service].state === "running" ? "stop" : "start")}>{data()!.services[service].state === "running" ? "Stop" : "Start"}</button></div></div>}</For>
          <Show when={host.can("tray")}>
            <div class="operation-service">
              <div>
                <strong>Launch at login</strong>
                <small>{autostart() ? "Starts in menu bar on boot" : "Manual launch only"}</small>
              </div>
              <div>
                <button
                  class="settings-button"
                  classList={{ primary: autostart() }}
                  onClick={() => void toggleAutostart()}
                >
                  {autostart() ? "Enabled" : "Disabled"}
                </button>
              </div>
            </div>
          </Show>
        </section>
        <section class="operation-card"><header><div><h3>Gateway</h3><p>Surfaces and approvals</p></div><Status value={data()!.gateway.enabled ? "enabled" : "disabled"} /></header>
          <div class="operation-facts"><span><b>Approvals</b>{data()!.gateway.approvals.mode}</span><span><b>Pending</b>{data()!.gateway.approvals.pending}</span><span><b>Bindings</b>{data()!.gateway.bindings.length}</span></div>
          <Show when={data()!.gateway.bindings.length} fallback={<p class="operation-muted">No surfaces are currently bound.</p>}><div class="operation-list"><For each={data()!.gateway.bindings}>{(binding) => <div><code>{binding.target}</code><small>{binding.session_id.slice(0, 8)}</small></div>}</For></div></Show>
        </section>
        <section class="operation-card"><header><div><h3>Flows</h3><p>Saved automation runs</p></div><span class="metric">{data()!.flows.reduce((sum, flow) => sum + flow.runs, 0)} runs</span></header>
          <Show when={data()!.flows.length} fallback={<p class="operation-muted">No flow runs discovered yet.</p>}><div class="operation-list"><For each={data()!.flows}>{(flow) => <div><strong>{flow.name}</strong><small>{flow.runs} {flow.runs === 1 ? "run" : "runs"}</small></div>}</For></div></Show>
        </section>
        <section class="operation-card doctor-card">
          <header><div><h3>Diagnostics</h3><p>System health</p></div>
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
            <Show when={doctor()!.ladder}>
              {(ladder) => (
                <div class="doctor-ladder">
                  <b>Frozen ladder · {ladder().objective} · {ladder().fallback_legs} fallback leg{ladder().fallback_legs === 1 ? "" : "s"}</b>
                  <code>{ladder().rendered}</code>
                  <Show when={ladder().annotations.length}>
                    <For each={ladder().annotations}>{(note) => <small>{note}</small>}</For>
                  </Show>
                </div>
              )}
            </Show>
          </Show>
        </section>
        <section class="operation-card finops-card"><header><div><h3>Spend & budget</h3><p>Estimated spend today</p></div><strong class="operation-cost">${finops()?.day_usd.toFixed(2) ?? "0.00"}</strong></header>
          <div class="operation-facts"><span><b>Day cap</b>{finops()?.day_cap_usd == null ? "Not set" : `$${finops()!.day_cap_usd!.toFixed(2)}`}</span><span><b>Run cap</b>{finops()?.run_cap_usd == null ? "Not set" : `$${finops()!.run_cap_usd!.toFixed(2)}`}</span><span><b>Calls</b>{finops()?.total_rows ?? 0}</span><span><b>Unknown price</b>{finops()?.unknown_rows ?? 0}</span></div>
          <div class="operation-rollups"><For each={finops()?.by_provider ?? []}>{(item) => <div><span>{item.name || "Unknown provider"}</span><small>${item.usd.toFixed(2)} · {item.calls} calls</small></div>}</For></div>
        </section>
      </div>
    </Show>
  </div>;
}
