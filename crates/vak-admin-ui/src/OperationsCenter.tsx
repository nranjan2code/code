import { For, Show, createMemo, createResource, createSignal, onCleanup } from "solid-js";
import { api } from "./api";
import { conn, navigate, pushToast, route, selectedAgentId, setSelectedAgentId } from "./store";
import type { OperationsSnapshot, SandboxRecord } from "./types";
import "./focusTrap";
import { cadence as automationCadence } from "./Automations";
import { EffectList } from "./Effects";

type Section = "overview" | "work" | "runtime" | "channels" | "automations" | "providers" | "incidents" | "sandbox";
type TimeWindow = "live" | "1h" | "24h" | "7d" | "custom";

const timeWindowLabels: Record<TimeWindow, string> = {
  live: "Live",
  "1h": "Last hour",
  "24h": "Last 24 hours",
  "7d": "Last 7 days",
  custom: "Custom window",
};

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function operationPath(value = route()): string {
  return value.split("?", 1)[0] || "#/operations";
}

function operationQuery(value = route()): URLSearchParams {
  const query = value.split("?", 2)[1];
  return new URLSearchParams(query || "");
}

function operationNavigate(path: string, changes: Record<string, string | null> = {}) {
  const params = operationQuery();
  for (const [key, value] of Object.entries(changes)) {
    if (value == null || value === "" || value === "all") params.delete(key);
    else params.set(key, value);
  }
  const query = params.toString();
  navigate(`${path}${query ? `?${query}` : ""}`);
}

function operationHref(path: string): string {
  const query = operationQuery().toString();
  return `${path}${query ? `?${query}` : ""}`;
}

function cutoffFor(windowName: TimeWindow): number | null {
  const now = Date.now();
  if (windowName === "1h") return now - 60 * 60 * 1000;
  if (windowName === "24h") return now - 24 * 60 * 60 * 1000;
  if (windowName === "7d") return now - 7 * 24 * 60 * 60 * 1000;
  return null;
}

const duration = (seconds: number) => {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
};

const time = (value: string | null | undefined) => {
  if (!value) return "—";
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? value : date.toLocaleString();
};

const tone = (value: string) => {
  if (["healthy", "running", "delivered", "pass"].includes(value)) return "good";
  if (["degraded", "pending", "waiting_approval", "warning", "stopped"].includes(value)) return "warn";
  if (["critical", "dead_letter", "fail", "not installed", "fenced"].includes(value)) return "bad";
  return "neutral";
};

function StatusMark(props: { value: string }) {
  return <span class={`ops-status ops-status-${tone(props.value)}`}><span class="ops-status-dot" />{props.value.replaceAll("_", " ")}</span>;
}

function Metric(props: { label: string; value: string | number; detail?: string; tone?: string }) {
  return <div class="ops-metric">
    <span class="eyebrow">{props.label}</span>
    <strong class={props.tone ? `text-${props.tone}` : ""}>{props.value}</strong>
    <Show when={props.detail}><span class="dim">{props.detail}</span></Show>
  </div>;
}

function workspaceOptions(data: OperationsSnapshot): string[] {
  const values = new Set<string>([data.server.cwd]);
  for (const binding of data.gateway.bindings) if (binding.workspace) values.add(binding.workspace);
  for (const entry of data.pool.entries) values.add(entry.workspace);
  for (const run of data.runs) values.add(run.workspace);
  for (const task of data.tasks) if (task.workspace) values.add(task.workspace);
  return [...values].filter(Boolean).sort((a, b) => a.localeCompare(b));
}

function workspaceLabel(path: string, data?: OperationsSnapshot): string {
  const named = data?.gateway.workspace_catalog?.find((entry) => entry.path === path)?.name;
  if (named) return named;
  const parts = path.replace(/\\/g, "/").split("/").filter(Boolean);
  return parts.at(-1) || path;
}

export function scopeOperations(data: OperationsSnapshot, scopeFilter: string, windowName: TimeWindow): OperationsSnapshot {
  const cutoff = cutoffFor(windowName);
  const isAgent = scopeFilter.startsWith("agent:");
  const targetAgent = isAgent ? scopeFilter.slice("agent:".length) : null;
  const inWorkspace = (value: string | null | undefined) => {
    if (!scopeFilter || scopeFilter === "all") return true;
    if (isAgent) return true;
    return value === scopeFilter;
  };
  const matchesAgent = (agentId: string | null | undefined) => {
    if (!isAgent || !targetAgent) return true;
    return (agentId || "vak") === targetAgent;
  };
  const inWindow = (value: string | number | null | undefined) => {
    if (cutoff == null || value == null) return true;
    const timestamp = typeof value === "number" ? value : Date.parse(value);
    return Number.isNaN(timestamp) || timestamp >= cutoff;
  };
  const bindings = data.gateway.bindings.filter((binding) => inWorkspace(binding.workspace) && matchesAgent(binding.agent_id));
  const pool = data.pool.entries.filter((entry) => inWorkspace(entry.workspace));
  const runs = data.runs.filter((run) => inWorkspace(run.workspace) && matchesAgent(run.agent_id));
  const pendingApprovals = runs.reduce((total, run) => total + run.pending_approvals.length, 0);
  const tasks = data.tasks.filter((task) => inWorkspace(task.workspace ?? "") && matchesAgent(task.agent));
  const records = data.effects.records.filter((record) => inWindow(record.updated_at));
  const security = data.security.filter((event) => inWindow(event.ts));
  const incidents = data.incidents.filter((incident) =>
    inWorkspace(incident.workspace || data.server.cwd) && inWindow(incident.last_seen),
  );
  return {
    ...data,
    gateway: { ...data.gateway, bindings, approvals: { ...data.gateway.approvals, pending: pendingApprovals } },
    pool: { ...data.pool, entries: pool },
    runs,
    tasks,
    effects: {
      ...data.effects,
      waiting: records.filter((record) => ["queued", "sending", "retrying"].includes(record.status)).length,
      held: records.filter((record) => record.status === "held").length,
      unknown: records.filter((record) => record.status === "unknown").length,
      failed: records.filter((record) => record.status === "failed").length,
      records,
    },
    security,
    incidents,
  };
}

function activeIncidents(data: OperationsSnapshot) {
  return data.incidents.filter((incident) => incident.status !== "resolved");
}

function OperationsContextBar(props: {
  data: OperationsSnapshot;
  workspace: string;
  timeWindow: TimeWindow;
  onAskDoctor: () => void;
}) {
  const [agentSearch, setAgentSearch] = createSignal("");
  const filteredAgents = createMemo(() => {
    const query = agentSearch().trim().toLocaleLowerCase();
    return (props.data.agents ?? [{ id: "vak", name: "Vakyartha" }])
      .filter((agent) => !query || `${agent.name} ${agent.id}`.toLocaleLowerCase().includes(query))
      .sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id));
  });
  const connectionLabel = () => conn() === "live" ? "Live stream" : conn() === "connecting" ? "Connecting" : "Disconnected";
  return <section class="ops-context-bar" aria-label="Operational context">
    <div class="ops-context-group">
      <label class="ops-context-field"><span>Scope</span><input class="ops-agent-search" type="search" aria-label="Find an agent" placeholder={`Find among ${(props.data.agents ?? []).length} agents`} value={agentSearch()} onInput={(event) => setAgentSearch(event.currentTarget.value)} /><select value={props.workspace || "all"} onChange={(event) => {
        const val = event.currentTarget.value;
        if (val === "all") {
          setSelectedAgentId("all");
          localStorage.setItem("vak_admin_selected_agent", "all");
        } else if (val.startsWith("agent:")) {
          const ag = val.slice("agent:".length);
          setSelectedAgentId(ag);
          localStorage.setItem("vak_admin_selected_agent", ag);
        }
        operationNavigate(operationPath(), { workspace: val });
      }}>
        <option value="all">All agents</option>
        <optgroup label={`Agents (${filteredAgents().length}${agentSearch() ? ` of ${(props.data.agents ?? []).length}` : ""})`}>
          <For each={filteredAgents()}>
            {(agent) => <option value={`agent:${agent.id}`}>{agent.name} ({agent.id})</option>}
          </For>
          <Show when={filteredAgents().length === 0}><option disabled>No agents match</option></Show>
        </optgroup>
        <optgroup label="Working Directories">
          <For each={workspaceOptions(props.data)}>{(workspace) => <option value={workspace}>{workspaceLabel(workspace, props.data)} · {workspace}</option>}</For>
        </optgroup>
      </select></label>
      <label class="ops-context-field"><span>Time</span><select value={props.timeWindow} onChange={(event) => operationNavigate(operationPath(), { time: event.currentTarget.value })}>
        <For each={Object.entries(timeWindowLabels)}>{([value, label]) => <option value={value}>{label}</option>}</For>
      </select></label>
    </div>
    <div class="ops-context-status">
      <span class={`ops-connection ops-connection-${conn()}`}><span class="ops-connection-dot" />{connectionLabel()}</span>
      <span class="mono dim">local · pid {props.data.server.pid}</span>
      <Show when={props.data.gateway.approvals.pending > 0}><button type="button" class="ops-context-link ops-context-warn" onClick={() => navigate(operationHref("#/operations/work"))}>{props.data.gateway.approvals.pending} approvals</button></Show>
      <Show when={activeIncidents(props.data).length > 0}><button type="button" class="ops-context-link ops-context-alert" onClick={() => navigate(operationHref("#/operations/incidents"))}>{activeIncidents(props.data).length} incidents</button></Show>
      <button type="button" class="ghost small" onClick={() => navigate(operationHref("#/search"))}>Search evidence</button>
      <button type="button" class="primary small" onClick={props.onAskDoctor}>Ask doctor</button>
    </div>
  </section>;
}

function Breadcrumbs(props: { path: string }) {
  const pieces = props.path.replace(/^#\//, "").split("/").filter(Boolean);
  return <nav class="ops-breadcrumbs" aria-label="Breadcrumb">
    <button type="button" class="link-button" onClick={() => navigate(operationHref("#/operations"))}>Operations</button>
    <For each={pieces.slice(1)}>{(piece, index) => <><span aria-hidden="true">/</span><span class={index() === pieces.length - 2 ? "current" : ""}>{piece.replaceAll("-", " ")}</span></>}</For>
  </nav>;
}

function Topology(props: { data: OperationsSnapshot; selected: string; onSelect: (id: string) => void }) {
  const nodes = [
    { id: "server", label: "Server", value: `${props.data.server.pid}`, detail: `PID · ${duration(props.data.server.uptime_secs)}` },
    { id: "gateway", label: "Gateway", value: `${props.data.gateway.bindings.length}`, detail: "active bindings" },
    { id: "pool", label: "CorePool", value: `${props.data.pool.entries.length}/${props.data.pool.max}`, detail: "warm / capacity" },
    { id: "work", label: "Work", value: `${props.data.runs.length}`, detail: "live runs" },
    { id: "delivery", label: "Delivery", value: `${props.data.effects.waiting}`, detail: `${props.data.effects.unknown} not sure, ${props.data.effects.failed} didn't send` },
    { id: "sandbox", label: "Sandbox", value: `${props.data.health.sandbox || "—"}`, detail: "execution backend" },
  ];
  return <section class="panel operations-topology-panel">
    <div class="panel-title-row">
      <div><span class="eyebrow">System map</span><h2>Operations topology</h2><p class="dim">Select a node to inspect the evidence behind the posture.</p></div>
      <StatusMark value={props.data.server.posture} />
    </div>
    <div class="operations-topology" role="list" aria-label="Operations topology">
      <svg class="operations-topology-lines" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
        <path d="M17 50 H83 M50 20 V80" />
      </svg>
      <For each={nodes}>{(node) => <button
        class={`operations-node operations-node-${node.id}`}
        classList={{ selected: props.selected === node.id }}
        role="listitem"
        aria-pressed={props.selected === node.id}
        onClick={() => props.onSelect(node.id)}
      >
        <span class="operations-node-label">{node.label}</span>
        <strong>{node.value}</strong>
        <span class="dim">{node.detail}</span>
      </button>}</For>
    </div>
    <div class="operations-inspector" aria-live="polite">
      <span class="eyebrow">Inspector · {props.selected}</span>
      <Show when={props.selected === "server"}><p>Process {props.data.server.pid} has been up for {duration(props.data.server.uptime_secs)} in <span class="mono">{props.data.server.cwd}</span>. Doctor posture is <StatusMark value={props.data.server.posture} />.</p></Show>
      <Show when={props.selected === "gateway"}><p>{props.data.gateway.bindings.length} active binding(s); {props.data.gateway.approvals.pending} approval gate(s). The gateway is <StatusMark value={props.data.gateway.enabled ? "enabled" : "disabled"} />.</p></Show>
      <Show when={props.selected === "pool"}><p>{props.data.pool.entries.length} warm Core instance(s) out of {props.data.pool.max}; idle eviction is {props.data.pool.idle_secs}s.</p></Show>
      <Show when={props.selected === "work"}><p>{props.data.runs.length} run(s) currently hold a live session handle. Open Live work for the approval and ledger trail.</p></Show>
      <Show when={props.selected === "delivery"}><p>{props.data.effects.waiting} message(s) still sending, {props.data.effects.unknown} not sure they were sent and {props.data.effects.failed} that didn't send.</p></Show>
      <Show when={props.selected === "sandbox"}><p>Execution backend: <StatusMark value={props.data.health.sandbox || "unset"} />. Staged, promoted, and quarantined environments live under <code>.vak/scratch/</code>. Open the Sandbox tab for the durable record ledger.</p></Show>
    </div>
  </section>;
}

function WorkView(props: { data: OperationsSnapshot }) {
  return <div class="operations-stack">
    <section class="panel">
      <div class="panel-title-row"><div><span class="eyebrow">Live work</span><h2>Runs and gates</h2><p class="dim">A run remains visible until its session ledger is returned to the handle.</p></div><Metric label="Active" value={props.data.runs.length} /></div>
      <Show when={props.data.runs.length > 0} fallback={<div class="empty">No active runs or pending approvals.</div>}>
        <div class="ops-table-wrap"><table class="ops-table"><thead><tr><th>Session</th><th>Agent</th><th>State</th><th>Workspace</th><th>Approvals</th><th /></tr></thead><tbody><For each={props.data.runs}>{(run) => <tr>
          <td><button class="link-button mono" onClick={() => navigate(liveRunHref(run))}>{run.session_id.slice(0, 12)}</button></td>
          <td><span class="chip chip-tone-info">✦ {run.agent_name || run.agent_id || "Vakyartha"}</span></td>
          <td><StatusMark value={run.state} /></td><td class="mono">{run.workspace}</td><td>{run.pending_approvals.length || "—"}</td>
          <td><button type="button" class="ghost small" onClick={() => navigate(liveRunHref(run))}>Open trail</button></td>
        </tr>}</For></tbody></table></div>
      </Show>
    </section>
    <section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Permission engine</span><h2>Approval gates</h2></div></div>
      <Show when={props.data.gateway.approvals.pending > 0} fallback={<div class="empty">No approval gates are waiting.</div>}>
        <div class="ops-approval-list"><For each={props.data.runs.flatMap((run) => run.pending_approvals.map((approval) => ({ ...approval, session_id: run.session_id })))}>{(approval) => <article class="ops-approval-row">
          <div><strong>{approval.tool}</strong><p class="dim">{approval.reason || "Permission engine requested a decision."}</p><span class="mono dim">{approval.session_id.slice(0, 12)} · {time(approval.requested_at)}</span></div>
          <button type="button" onClick={() => navigate(`#/sessions/${approval.session_id}`)}>Review</button>
        </article>}</For></div>
      </Show>
    </section>
    <section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Unattended work</span><h2>Automations</h2><p class="dim">Each with its next run and in-flight state; its last run is read from the run records.</p></div><button type="button" class="ghost small" onClick={() => navigate(operationHref("#/operations/automations"))}>Open automations</button></div>
      <Show when={props.data.tasks.length > 0} fallback={<div class="empty">No automations are set up.</div>}>
        <div class="ops-table-wrap"><table class="ops-table"><thead><tr><th>Name</th><th>Kind</th><th>State</th><th>Next run</th><th>Last run</th></tr></thead><tbody><For each={props.data.tasks}>{(task) => <tr>
          <td><strong>{task.name}</strong><p class="mono dim">{task.workspace || "another machine"}</p></td>
          <td>{task.action.kind}<Show when={task.action.kind === "prompt" && task.action.model_pin}><p class="mono dim">{task.action.kind === "prompt" ? task.action.model_pin : ""}</p></Show></td>
          <td><StatusMark value={task.running ? "running" : task.enabled ? "enabled" : "disabled"} /></td>
          <td>{task.next_run_at ? time(task.next_run_at) : automationCadence(task)}</td>
          <td>{time(task.last_run?.opened_at)}</td>
        </tr>}</For></tbody></table></div>
      </Show>
    </section>
  </div>;
}

function AutomationsView(props: { data: OperationsSnapshot }) {
  return <div class="operations-stack">
    <section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Operations</span><h2>Automations</h2><p class="dim">Every automation with its actual workspace, cadence, model pin and last run, read from the run records.</p></div><button class="ghost small" onClick={() => navigate("#/automations")}>Manage automations</button></div>
      <Show when={props.data.tasks.length > 0} fallback={<div class="empty">No automations are set up for this scope. Add one to make unattended work visible here.</div>}>
        <div class="ops-table-wrap"><table class="ops-table"><thead><tr><th>Automation</th><th>Workspace</th><th>Cadence</th><th>State</th><th>Model pin</th><th>Last run</th><th /></tr></thead><tbody><For each={props.data.tasks}>{(item) => <tr>
          <td><strong>{item.name}</strong><p class="mono dim">{item.id.slice(0, 16)}</p></td>
          <td class="mono">{item.workspace || "another machine"}</td>
          <td>{item.next_run_at ? `next ${time(item.next_run_at)}` : automationCadence(item)}</td>
          <td><StatusMark value={item.running ? "running" : item.enabled ? "enabled" : "disabled"} /></td>
          <td class="mono">{item.action.kind === "prompt" ? item.action.model_pin || "inherits" : "—"}</td>
          <td>{item.last_run ? <button class="link-button" onClick={() => navigate(`#/runs/${encodeURIComponent(item.last_run!.id)}`)}>{item.last_run.status} · {time(item.last_run.opened_at)}</button> : "never"}</td>
          <td><button class="ghost small" onClick={() => navigate("#/automations")}>Inspect</button></td>
        </tr>}</For></tbody></table></div>
      </Show>
    </section>
  </div>;
}

function RuntimeView(props: { data: OperationsSnapshot; act: (service: "gateway" | "bridges", action: "start" | "stop" | "restart") => void; acting: string | null }) {
  const services = [{ id: "gateway" as const, label: "Gateway", info: props.data.services.gateway }, { id: "bridges" as const, label: "Chat bridges", info: props.data.services.bridges }];
  return <div class="operations-stack">
    <Show when={selectedAgentId() !== "global"}>
      <div style="display:flex; align-items:center; justify-content:space-between; margin-bottom:14px; padding:12px 16px; background:var(--surface-raised); border:1px solid var(--border); border-radius:var(--radius-sm); font-size:13px;">
        <div>
          <strong>🌐 Host Platform Services</strong>
          <p class="dim" style="margin:2px 0 0;">
            Daemon lifecycle and event fabric run at the platform level. Actions here apply to the host rather than an individual agent.
          </p>
        </div>
        <button class="ghost small" onClick={() => { setSelectedAgentId("all"); localStorage.setItem("vak_admin_selected_agent", "all"); }}>
          Switch to Global
        </button>
      </div>
    </Show>
    <section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Runtime</span><h2>Service manager</h2><p class="dim">Manager state and HTTP health are separate signals. Actions delegate to vak-ops.</p></div><StatusMark value={props.data.services.gateway_healthy ? "healthy" : "degraded"} /></div>
      <div class="operations-service-grid"><For each={services}>{(service) => <article class="operations-service-card"><div class="panel-title-row"><div><h3>{service.label}</h3><StatusMark value={service.info.state} /></div><span class="mono dim">port {props.data.ops_port}</span></div><div class="ops-action-row"><button class="ghost small" disabled={props.acting !== null} onClick={() => props.act(service.id, "start")}>Start</button><button class="ghost small" disabled={props.acting !== null} onClick={() => props.act(service.id, "restart")}>Restart</button><button class="ghost small" disabled={props.acting !== null} onClick={() => props.act(service.id, "stop")}>Stop</button></div><Show when={props.acting === service.id}><span class="dim">Applying manager action…</span></Show></article>}</For></div>
    </section>
    <section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Warm workers</span><h2>CorePool occupancy</h2><p class="dim">Each row is a real workspace plus its restrictive permission override.</p></div><Metric label="Capacity" value={`${props.data.pool.entries.length}/${props.data.pool.max}`} /></div>
      <Show when={props.data.pool.entries.length > 0} fallback={<div class="empty">The pool is cold. The next approved inbound will start a workspace Core.</div>}>
        <div class="ops-pool-list"><For each={props.data.pool.entries}>{(entry) => <div class="ops-pool-row"><div class="ops-pool-name"><strong>{entry.workspace}</strong><span class="dim">{entry.is_default ? "default workspace" : "pooled workspace"}</span></div><div class="ops-pool-bar"><span style={{ width: `${Math.max(5, Math.min(100, 100 - (entry.idle_secs / Math.max(1, props.data.pool.idle_secs)) * 100))}%` }} /></div><div class="mono">{entry.idle_secs}s idle</div><StatusMark value={entry.effective_permission_mode} /></div>}</For></div>
      </Show>
    </section>
    <Show when={props.data.bus !== null}>
      <section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Event fabric</span><h2>Distributed bus</h2><p class="dim">vak-bus (docs/design/53): NATS Core + JetStream with AES-256-GCM envelope security and W3C/Merkle causal lineage.</p></div><StatusMark value={props.data.bus!.connected ? "healthy" : "degraded"} /></div>
        <div class="operations-metric-grid">
          <Metric label="Backend" value={props.data.bus!.backend} />
          <Metric label="Connected" value={props.data.bus!.connected ? "yes" : "no"} />
          <Metric label="Encrypted" value={props.data.bus!.encrypted ? "yes" : "no"} />
          <Metric label="Workspace ID" value={props.data.bus!.workspace_id.slice(0, 8) + "…"} />
        </div>
        <div class="operations-metric-grid" style="margin-top:8px">
          <Metric label="Published" value={props.data.bus!.metrics.published_count} />
          <Metric label="Received" value={props.data.bus!.metrics.received_count} />
          <Metric label="Bytes out" value={formatBytes(props.data.bus!.metrics.bytes_published)} />
          <Metric label="Bytes in" value={formatBytes(props.data.bus!.metrics.bytes_received)} />
          <Metric label="Dead-lettered" value={props.data.bus!.metrics.dead_letter_count} tone={props.data.bus!.metrics.dead_letter_count > 0 ? "bad" : undefined} />
          <Metric label="Queue lag" value={props.data.bus!.metrics.active_queue_lag} />
        </div>
      </section>
    </Show>
    <section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Action ledger</span><h2>Recent receipts</h2><p class="dim">Every service or delivery mutation records the request, post-action probe, and persistence result.</p></div><Metric label="Stored" value={props.data.actions.length} /></div>
      <Show when={props.data.actions.length > 0} fallback={<div class="empty">No operational actions have been recorded.</div>}>
        <div class="ops-event-list"><For each={props.data.actions.slice(0, 12)}>{(receipt) => <article><div><StatusMark value={receipt.verification.status} /><strong>{receipt.service} · {receipt.action}</strong><span class="mono dim">{receipt.receipt_id}</span></div><span class="mono dim">{time(receipt.completed_at)}</span><p>{receipt.verification.detail} <span class="mono">{receipt.verification.before} → {receipt.verification.after}</span></p></article>}</For></div>
      </Show>
    </section>
  </div>;
}

function ChannelsView(props: { data: OperationsSnapshot; refresh: () => void | Promise<unknown> }) {
  return <div class="operations-stack"><section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Channels</span><h2>Gateway bindings</h2><p class="dim">Identity, workspace, and frozen route provenance for each bound target.</p></div><StatusMark value={props.data.gateway.enabled ? "enabled" : "disabled"} /></div>
    <Show when={props.data.gateway.bindings.length > 0} fallback={<div class="empty">No channel has an active binding yet.</div>}><div class="ops-binding-grid"><For each={props.data.gateway.bindings}>{(binding) => <article class="ops-binding-card"><div class="panel-title-row"><strong class="mono">{binding.target}</strong><Show when={binding.agent_id}><span class="chip chip-tone-info">✦ {binding.agent_id}</span></Show><Show when={binding.session_id}><span class="chip chip-tone-info">bound</span></Show></div><dl><div><dt>Workspace</dt><dd class="mono">{binding.workspace || "inherited"}</dd></div><div><dt>Route</dt><dd>{binding.provider || "—"} / <span class="mono">{binding.model || "—"}</span></dd></div><div><dt>Revision</dt><dd class="mono">{binding.route_revision || "—"}</dd></div></dl><button class="ghost small" onClick={() => navigate(operationHref(`#/operations/channels/${encodeURIComponent(binding.target)}`))}>Open binding trail</button></article>}</For></div></Show>
  </section><section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Delivery</span><h2>Messages sent</h2><p class="dim">Each message is recorded before it is sent. One nobody can prove arrived is never sent again by itself: send it again, or say whether it arrived.</p></div><div class="ops-summary-pair"><Metric label="Sending" value={props.data.effects.waiting} /><Metric label="Not sure" value={props.data.effects.unknown} tone={props.data.effects.unknown ? "bad" : undefined} /><Metric label="Didn't send" value={props.data.effects.failed} tone={props.data.effects.failed ? "bad" : undefined} /></div></div><Show when={props.data.effects.error}><div class="error-state" role="alert"><strong>Sent messages could not be read</strong><p>{props.data.effects.error}</p></div></Show><EffectList effects={props.data.effects.records} onChanged={props.refresh} /></section></div>;
}

function ProvidersView(props: { data: OperationsSnapshot }) {
  return <div class="operations-stack"><section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Route</span><h2>Provider posture</h2><p class="dim">The effective route is frozen per session; this view shows the current workspace route.</p></div><StatusMark value={props.data.health.posture || props.data.health.status} /></div><div class="operations-metric-grid"><Metric label="Provider" value={props.data.health.provider} detail={props.data.health.provider_source} /><Metric label="Model" value={props.data.health.model} detail={props.data.health.model_source} /><Metric label="Permission" value={props.data.health.permission_mode} /><Metric label="Sandbox" value={props.data.health.sandbox} /></div><div class="ops-facts"><For each={props.data.health.facts}>{(fact) => <p>{fact}</p>}</For></div></section><section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Spend and failure evidence</span><h2>Recent security events</h2></div></div><Show when={props.data.security.length > 0} fallback={<div class="empty">No security events recorded.</div>}><div class="ops-event-list"><For each={props.data.security}>{(event) => <article><div><StatusMark value={event.kind} /><strong>{event.label}</strong></div><span class="mono dim">{time(event.ts)}</span><p>{event.detail}</p></article>}</For></div></Show></section></div>;
}

function IncidentsView(props: { data: OperationsSnapshot }) {
  const open = () => activeIncidents(props.data);
  const resolved = () => props.data.incidents.filter((incident) => incident.status === "resolved");
  return <div class="operations-stack"><section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Causal queue</span><h2>Incidents and doctor evidence</h2><p class="dim">These are derived from failed checks, blocked work, delivery state, and service-manager truth. Open one to preserve the investigation scope.</p></div><Metric label="Open" value={open().length} /></div><Show when={open().length > 0} fallback={<div class="empty">No active incidents. The absence is based on current probes, not a placeholder green state.</div>}><div class="ops-incident-list"><For each={open()}>{(incident) => <button class={`ops-incident ops-incident-${tone(incident.severity)}`} onClick={() => navigate(operationHref(`#/operations/incidents/${encodeURIComponent(incident.id)}`))}><div class="ops-incident-marker" /><div><div class="panel-title-row"><strong>{incident.title}</strong><StatusMark value={incident.severity} /></div><span class="eyebrow">{incident.source} · {incident.id}</span><p>{incident.detail}</p><span class="ops-incident-link">Open timeline and evidence →</span></div></button>}</For></div></Show></section><Show when={resolved().length > 0}><section class="panel"><div class="panel-title-row"><div><span class="eyebrow">History</span><h2>Resolved incidents</h2><p class="dim">Resolution is retained as evidence; it is not silently deleted.</p></div><Metric label="Resolved" value={resolved().length} /></div><div class="ops-event-list"><For each={resolved()}>{(incident) => <article><div><StatusMark value="resolved" /><strong>{incident.title}</strong><span class="mono dim">{incident.id}</span></div><span class="mono dim">{time(incident.last_seen)}</span><p>{incident.resolution || "Resolved by current probes."}</p><button class="ghost small" onClick={() => navigate(operationHref(`#/operations/incidents/${encodeURIComponent(incident.id)}`))}>Open retained evidence</button></article>}</For></div></section></Show><section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Doctor checks</span><h2>Evidence</h2></div></div><div class="ops-check-list"><For each={props.data.health.checks}>{(check) => <div class="ops-check-row"><StatusMark value={check.status} /><strong>{check.label}</strong><span>{check.detail}</span></div>}</For></div></section></div>;
}

type LiveRun = OperationsSnapshot["runs"][number];

/** Where live work links: its run when one is recorded, else its conversation. */
export function liveRunHref(run: { session_id: string; run_id?: string | null }): string {
  return run.run_id ? `#/runs/${encodeURIComponent(run.run_id)}` : `#/sessions/${encodeURIComponent(run.session_id)}`;
}

/** The evidence trail of one conversation a run wrote: ledger, managed
 *  work, provider receipts and sandbox executions. */
export function SessionTrail(props: { sessionId: string; live?: LiveRun }) {
  const [transcript, transcriptState] = createResource(() => props.sessionId, (id) => api.transcript(id, { limit: 200, refresh: true }));
  const [voiceQuery, setVoiceQuery] = createSignal("");
  const [voiceStatus, setVoiceStatus] = createSignal<"all" | "success" | "error">("all");
  const [receipts, receiptsState] = createResource(() => props.sessionId, (id) => api.receipts(id));
  const [work, { refetch: refetchWork }] = createResource(() => props.sessionId, (id) => api.work(id));
  const [sandboxExecs, sandboxState] = createResource(() => props.sessionId, (id) => api.sessionSandboxExecutions(id));
  // Derived strictly from append-only ledger entries; absent events remain absent.
  const voiceEntries = createMemo(() => {
    const query = voiceQuery().trim().toLowerCase();
    return (transcript()?.entries ?? []).filter((entry) => {
      if (!/voicetranscript|voiceplayback/i.test(entry.kind)) return false;
      if (voiceStatus() === "error" && !entry.is_error) return false;
      if (voiceStatus() === "success" && entry.is_error) return false;
      return !query || [entry.kind, entry.role ?? "", entry.tool_name ?? "", entry.content]
        .join(" ").toLowerCase().includes(query);
    });
  });
  const displayedEntries = createMemo(() => voiceQuery().trim() || voiceStatus() !== "all" ? voiceEntries() : (transcript()?.entries ?? []));
  const run = () => props.live;
  const command = async (value: Record<string, unknown>) => {
    try { await api.workCommand(props.sessionId, value); await refetchWork(); pushToast("info", "Work ledger updated"); }
    catch (error) { pushToast("alert", `Work update failed: ${error}`); }
  };
  return <div class="operations-stack"><section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Conversation</span><h2 class="mono">{props.sessionId}</h2><p class="dim">A preserved trail from live handle to session ledger and provider receipts.</p></div><StatusMark value={run()?.state || "historical"} /></div><div class="operations-metric-grid"><Metric label="Workspace" value={run()?.workspace || "ledger"} /><Metric label="Approvals" value={run()?.pending_approvals.length || 0} /><Metric label="Ledger" value={transcript.loading ? "loading" : transcript.error ? "unavailable" : "available"} /><Metric label="Receipts" value={receipts.loading ? "loading" : receipts()?.length || 0} /></div><div class="ops-action-row"><button class="ghost small" onClick={() => navigate(`#/sessions/${encodeURIComponent(props.sessionId)}`)}>Open session console</button></div></section><Show when={work()}>{(projection) => <section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Managed work</span><h2>{projection().contract.objective}</h2><p class="dim">Contract <span class="mono">{projection().contract.contract_id}</span> · {projection().status}</p></div><StatusMark value={projection().status} /></div><div class="ops-check-list"><For each={Object.entries(projection().items)}>{([itemId, item]) => <div class="ops-check-row"><StatusMark value={item.status} /><strong class="mono">{itemId}</strong><span>attempt {item.attempt}{item.blocker ? ` · ${item.blocker}` : ""}</span><Show when={item.status === "failed" || item.status === "interrupted"}><button class="ghost small" onClick={() => void command({ operation: "retry", item_id: itemId, reason: "operator retry" })}>Retry</button></Show></div>}</For></div><Show when={projection().status === "blocked"}><div class="ops-action-row"><button class="primary small" onClick={() => void command({ operation: "resume", reason: "operator resumed contract" })}>Resume contract</button></div></Show></section>}</Show><section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Timeline</span><h2>Ledger entries</h2><p class="dim">Chronological evidence from the append-only session log.</p><div class="ops-action-row"><input aria-label="Search voice events" placeholder="Search voice events" value={voiceQuery()} onInput={(event) => setVoiceQuery(event.currentTarget.value)} /><select aria-label="Voice event status" value={voiceStatus()} onChange={(event) => setVoiceStatus(event.currentTarget.value as "all" | "success" | "error")}><option value="all">All outcomes</option><option value="success">Successful</option><option value="error">Errors</option></select></div></div></div><Show when={!transcript.loading} fallback={<div class="empty">Reading session ledger…</div>}><Show when={!transcript.error} fallback={<div class="error-state"><strong>Ledger unavailable</strong><p>{String(transcript.error)}</p></div>}><Show when={(transcript()?.entries.length ?? 0) > 0} fallback={<div class="empty">No ledger entries were returned.</div>}><div class="ops-timeline"><For each={displayedEntries()}>{(entry) => <article classList={{ "ops-timeline-error": entry.is_error }}><span class="ops-timeline-dot" /><div><div class="panel-title-row"><strong>{entry.kind}</strong><span class="mono dim">{time(entry.ts)}</span></div><p class="dim">{entry.role || "system"}{entry.tool_name ? ` · ${entry.tool_name}` : ""}</p><pre class="mono ops-evidence-pre">{entry.content}</pre></div></article>}</For></div></Show></Show></Show></section><section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Provider evidence</span><h2>Receipts</h2><p class="dim">Frozen provider/model attribution and settlement data.</p></div></div><Show when={!receipts.loading} fallback={<div class="empty">Reading receipts…</div>}><Show when={(receipts()?.length ?? 0) > 0} fallback={<div class="empty">No provider receipts recorded for this run.</div>}><div class="ops-receipt-list"><For each={receipts() ?? []}>{(receipt) => <pre class="mono ops-receipt">{JSON.stringify(receipt, null, 2)}</pre>}</For></div></Show></Show></section><Show when={!sandboxExecs.loading} fallback={<div class="empty">No sandbox executions for this session.</div>}><Show when={!sandboxExecs.error} fallback={<div class="error-state"><strong>Sandbox executions unavailable</strong><p>{String(sandboxExecs.error)}</p></div>}><Show when={(sandboxExecs()?.events?.length ?? 0) > 0}><section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Sandbox executions</span><h2>Per-session event log</h2><p class="dim">Streaming stdout, stderr, telemetry, and status from quarantined runs in <code>.vak/scratch/</code>.</p></div></div><div class="ops-timeline"><For each={sandboxExecs()?.events ?? []}>{(event) => <article><span class="ops-timeline-dot" /><div><div class="panel-title-row"><strong class="mono">{(event as any).event || "event"}</strong><span class="mono dim">{time((event as any).ts)}</span></div><pre class="mono ops-evidence-pre">{JSON.stringify(event, null, 2)}</pre></div></article>}</For></div></section></Show></Show></Show></div>;
}

function BindingDetail(props: { data: OperationsSnapshot; target: string }) {
  const binding = () => props.data.gateway.bindings.find((item) => item.target === props.target);
  const activeRun = () => {
    const sessionId = binding()?.session_id;
    return sessionId ? props.data.runs.find((run) => run.session_id === sessionId) : undefined;
  };
  return <div class="operations-stack"><Breadcrumbs path={operationPath()} /><section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Channel binding</span><h2 class="mono">{props.target}</h2><p class="dim">Identity → workspace → frozen route → session → delivery.</p></div><StatusMark value={binding() ? "bound" : "not found"} /></div><Show when={binding()} fallback={<div class="error-state"><strong>Binding not found</strong><p>This target is no longer present in the current gateway snapshot.</p></div>}>{(item) => <><dl class="ops-detail-grid"><div><dt>Workspace</dt><dd class="mono">{item().workspace || "inherited"}</dd></div><div><dt>Provider</dt><dd>{item().provider || "—"}</dd></div><div><dt>Model</dt><dd class="mono">{item().model || "—"}</dd></div><div><dt>Route revision</dt><dd class="mono">{item().route_revision || "—"}</dd></div><div><dt>Session</dt><dd class="mono">{item().session_id || "cold / next inbound"}</dd></div></dl><div class="ops-action-row"><Show when={item().session_id}><button class="ghost small" onClick={() => navigate(`#/sessions/${encodeURIComponent(item().session_id!)}`)}>Open conversation</button></Show><button class="ghost small" onClick={() => navigate("#/gateway")}>Edit binding</button></div><Show when={activeRun()}>{(run) => <div class="ops-related"><strong>Related live work</strong><p>{run().state} in <span class="mono">{run().workspace}</span> · {run().pending_approvals.length} approval gate(s).</p></div>}</Show></>}</Show></section><section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Dependencies</span><h2>What this binding touches</h2></div></div><div class="ops-dependency-chain"><span>channel identity</span><span>→</span><span class="mono">{props.target}</span><span>→</span><span>{binding()?.workspace || "workspace default"}</span><span>→</span><span>{binding()?.session_id || "next session"}</span><span>→</span><span>sent messages</span></div></section></div>;
}

function IncidentDetail(props: { data: OperationsSnapshot; incidentId: string; onAskDoctor: () => void }) {
  const incident = () => props.data.incidents.find((item) => item.id === props.incidentId);
  const relatedRuns = () => props.data.runs.filter((run) => incident()?.fingerprint === "permission:pending-approvals" ? run.pending_approvals.length > 0 : incident()?.workspace ? run.workspace === incident()?.workspace : true);
  return <div class="operations-stack"><Breadcrumbs path={operationPath()} /><section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Incident</span><h2>{incident()?.title || props.incidentId}</h2><p class="dim">{incident()?.detail || "This incident is no longer present in the current posture."}</p></div><StatusMark value={incident()?.status === "resolved" ? "resolved" : incident()?.severity || "unknown"} /></div><dl class="ops-detail-grid"><div><dt>Incident id</dt><dd class="mono">{props.incidentId}</dd></div><div><dt>Source</dt><dd>{incident()?.source || "historical"}</dd></div><div><dt>Scope</dt><dd class="mono">{incident()?.workspace || props.data.server.cwd}</dd></div><div><dt>First seen</dt><dd>{time(incident()?.first_seen)}</dd></div><div><dt>Last seen</dt><dd>{time(incident()?.last_seen || props.data.generated_at)}</dd></div><div><dt>Occurrences</dt><dd>{incident()?.occurrences ?? "—"}</dd></div></dl><Show when={incident()?.resolution}><div class="ops-related"><strong>Resolution</strong><p>{incident()?.resolution}</p></div></Show><div class="ops-action-row"><button class="primary small" onClick={props.onAskDoctor}>Ask doctor about this incident</button><button class="ghost small" onClick={() => navigate(operationHref("#/operations/incidents"))}>Back to incidents</button></div></section><section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Correlated evidence</span><h2>What is affected now</h2></div></div><Show when={relatedRuns().length > 0} fallback={<div class="empty">No live runs are correlated with this incident.</div>}><div class="ops-related-list"><For each={relatedRuns()}>{(run) => <button class="ops-related-row" onClick={() => navigate(liveRunHref(run))}><span class="mono">{run.session_id.slice(0, 12)}</span><StatusMark value={run.state} /><span>{run.pending_approvals.length} approval gates</span><span class="mono dim">{run.workspace}</span></button>}</For></div></Show><Show when={incident()?.evidence?.length}><div class="ops-check-list"><For each={incident()?.evidence}>{(evidence) => <div class="ops-check-row"><StatusMark value="evidence" /><span class="mono">{evidence}</span></div>}</For></div></Show><div class="ops-check-list"><For each={props.data.health.checks}>{(check) => <div class="ops-check-row"><StatusMark value={check.status} /><strong>{check.label}</strong><span>{check.detail}</span></div>}</For></div></section></div>;
}

function SandboxView(props: { data: OperationsSnapshot; refresh: () => void | Promise<unknown> }) {
  const [records, { refetch: refetchRecords }] = createResource(() => api.sandboxRecords());
  const environments = createMemo(
    () => (records()?.records ?? []).filter((r): r is Extract<SandboxRecord, { kind: "Environment" }> => r.kind === "Environment"),
  );
  const candidates = createMemo(
    () => (records()?.records ?? []).filter((r): r is Extract<SandboxRecord, { kind: "Candidate" }> => r.kind === "Candidate"),
  );
  const promotions = createMemo(
    () => (records()?.records ?? []).filter((r): r is Extract<SandboxRecord, { kind: "Promotion" }> => r.kind === "Promotion"),
  );

  const refresh = async () => {
    await Promise.all([refetchRecords(), props.refresh()]);
  };

  return <div class="operations-stack">
    <section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Environments</span><h2>Sandbox environments</h2><p class="dim">Quarantined execution environments in <code>.vak/scratch/</code>. Each streams live stdout, stderr, telemetry, and status to the Workbench panel.</p></div><button class="ghost small" onClick={refresh}>Refresh</button></div>
      <Show when={environments().length > 0} fallback={<div class="empty">No sandbox environments have been staged yet.</div>}>
        <div class="ops-table-wrap"><table class="ops-table"><thead><tr><th>ID</th><th>State</th><th>Backend</th><th>Image</th><th>Network policy</th><th>Updated</th></tr></thead><tbody><For each={environments()}>{(env) => <tr>
          <td class="mono dim">{env.record.record_id}</td>
          <td><StatusMark value={env.record.state} /></td>
          <td><code>{env.record.plan.backend}</code></td>
          <td class="mono">{env.record.plan.image || "—"}</td>
          <td>{env.record.plan.network_policy}</td>
          <td class="mono dim">{time(env.record.updated_at)}</td>
        </tr>}</For></tbody></table></div>
      </Show>
    </section>

    <section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Candidates</span><h2>Exported sandbox candidates</h2><p class="dim">Drafts stay bound to the conversation result that produced them. Review and acceptance happen in that conversation; this view is the evidence ledger.</p></div><button class="ghost small" onClick={refresh}>Refresh</button></div>
      <Show when={candidates().length > 0} fallback={<div class="empty">No exported candidates yet. Run something in the sandbox to produce one.</div>}>
        <div class="ops-table-wrap"><table class="ops-table"><thead><tr><th>Candidate ID</th><th>Session / result</th><th>Verified</th><th>Files</th><th>Updated</th></tr></thead><tbody><For each={candidates()}>{(cand) => <tr>
          <td class="mono dim">{cand.record.candidate.candidate_id}</td>
          <td class="mono dim">{cand.record.session_id.slice(0, 10)} / {cand.record.result_id.slice(0, 10)}</td>
          <td><StatusMark value={cand.record.verified ? "pass" : "fail"} /></td>
          <td class="mono dim">{cand.record.candidate.files.length}</td>
          <td class="mono dim">{time(cand.record.updated_at)}</td>
        </tr>}</For></tbody></table></div>
      </Show>
    </section>

    <section class="panel"><div class="panel-title-row"><div><span class="eyebrow">Promotion ledger</span><h2>Promotion receipts</h2><p class="dim">Append-only record of every candidate promoted from sandbox to workspace, with hash verification evidence.</p></div><button class="ghost small" onClick={refresh}>Refresh</button></div>
      <Show when={promotions().length > 0} fallback={<div class="empty">No promotions have been recorded yet.</div>}>
        <div class="ops-event-list"><For each={promotions()}>{(p) => <article><div><strong>{p.record.candidate_id.slice(0, 12)}</strong><span class="mono dim"> · {p.record.receipt.applied.length} files applied</span></div><span class="mono dim">{time(p.record.updated_at)}</span><p class="mono dim">session: {p.record.session_id} · result: {p.record.result_id} · record: {p.record.record_id}</p>
          <Show when={p.record.receipt.verification.length > 0}>
            <div class="ops-check-list"><For each={p.record.receipt.verification}>{(v) => <div class="ops-check-row"><StatusMark value={v.status} /><strong>{v.path}</strong><span>{v.evidence}</span></div>}</For></div>
          </Show>
        </article>}</For></div>
      </Show>
    </section>
  </div>;
}

export function OperationsCenter(props: { section?: Section }) {
  const [snapshot, { refetch }] = createResource(() => api.operations());
  const [selected, setSelected] = createSignal("server");
  const [acting, setActing] = createSignal<string | null>(null);
  const [doctorOpen, setDoctorOpen] = createSignal(false);
  const [doctorLoading, setDoctorLoading] = createSignal(false);
  const [doctorResult, setDoctorResult] = createSignal<{ report: string; failures?: number; checks?: Array<{ label: string; ok: boolean; detail: string }> } | null>(null);
  const section = createMemo<Section>(() => {
    const current = operationPath();
    if (current === "#/operations/work" || current.startsWith("#/operations/work/")) return "work";
    if (current === "#/operations/runtime") return "runtime";
    if (current === "#/operations/channels" || current.startsWith("#/operations/channels/")) return "channels";
    if (current === "#/operations/automations") return "automations";
    if (current === "#/operations/providers") return "providers";
    if (current === "#/operations/incidents" || current.startsWith("#/operations/incidents/")) return "incidents";
    if (current === "#/operations/sandbox" || current.startsWith("#/operations/sandbox/")) return "sandbox";
    return props.section ?? "overview";
  });
  const workspace = createMemo(() => {
    const fromQuery = operationQuery().get("workspace");
    if (fromQuery) return fromQuery;
    const currentAgent = selectedAgentId();
    if (currentAgent && currentAgent !== "global" && currentAgent !== "all") {
      return `agent:${currentAgent}`;
    }
    return "all";
  });
  const timeWindow = createMemo<TimeWindow>(() => {
    const value = operationQuery().get("time");
    return value && value in timeWindowLabels ? value as TimeWindow : "live";
  });
  const scopedSnapshot = createMemo(() => {
    const value = snapshot();
    return value ? scopeOperations(value, workspace(), timeWindow()) : undefined;
  });
  const timer = window.setInterval(() => {
    if (document.visibilityState === "visible") refetch();
  }, 8000);
  onCleanup(() => window.clearInterval(timer));
  const act = async (service: "gateway" | "bridges", action: "start" | "stop" | "restart") => {
    setActing(service);
    try {
      const result = await api.opsAction(service, action);
      const verification = result.verification ? ` · ${result.verification.status} (${result.receipt_id || "no receipt"})` : "";
      pushToast(result.ok ? "info" : "alert", result.ok ? `${service} ${action} requested${verification}` : result.error || `${service} ${action} failed`);
      await refetch();
    } catch (error) {
      pushToast("alert", `${service} ${action} failed: ${error}`);
    } finally {
      setActing(null);
    }
  };
  const askDoctor = async () => {
    setDoctorOpen(true);
    setDoctorLoading(true);
    const sessionMatch = operationPath().match(/\/runs\/([^/]+)/);
    try {
      const result = await api.doctor(sessionMatch ? decodeURIComponent(sessionMatch[1]) : undefined);
      setDoctorResult({ ...result, report: result.report || JSON.stringify(result, null, 2) });
    } catch (error) {
      setDoctorResult({ report: `Doctor could not complete: ${error}` });
    } finally {
      setDoctorLoading(false);
    }
  };
  const title = () => ({ overview: "Operations Center", work: "Live work", runtime: "Runtime and pools", channels: "Channels and delivery", automations: "Automations", providers: "Providers and spend", incidents: "Incidents", sandbox: "Sandbox executions" }[section()] ?? "Operations Center");
  const path = createMemo(() => operationPath());
  const detailBinding = createMemo(() => path().startsWith("#/operations/channels/") ? decodeURIComponent(path().slice("#/operations/channels/".length)) : null);
  const detailIncident = createMemo(() => path().startsWith("#/operations/incidents/") ? decodeURIComponent(path().slice("#/operations/incidents/".length)) : null);
  const hasDetail = createMemo(() => Boolean(detailBinding() || detailIncident()));
  return <div class="view operations-view">
    <header class="page-header"><div><span class="eyebrow">Control plane</span><h1>{title()}</h1><p class="dim">{scopedSnapshot()?.server.cwd || "Loading operational scope…"}</p></div><div class="operations-header-meta"><Show when={scopedSnapshot()}>{(data) => <><StatusMark value={data().server.posture} /><span class="mono dim">updated {time(data().generated_at)}</span></>}</Show><button class="ghost small" onClick={() => refetch()}>Refresh</button></div></header>
    <Show when={scopedSnapshot() || snapshot.error} fallback={<div class="panel operations-loading"><div class="skel skel-line" /><div class="skel skel-block" /><p class="dim">Reading live ledgers and service probes…</p></div>}>
      <Show when={scopedSnapshot()} fallback={<div class="panel error-state"><strong>Operations snapshot unavailable</strong><p>{String(snapshot.error)}</p><button onClick={() => refetch()}>Retry</button></div>}>
        <Show when={scopedSnapshot()}>{(data) => <>
          <OperationsContextBar data={data()} workspace={workspace()} timeWindow={timeWindow()} onAskDoctor={() => void askDoctor()} />
          <Show when={path() !== "#/operations" && !hasDetail()}><Breadcrumbs path={path()} /></Show>
          <Show when={data().server.posture === "fenced"}><p class="ops-fenced-note">The data was restored after this server started, so it has stopped its scheduled work and deliveries and takes no new conversations. Restart it to continue.</p></Show><div class="operations-metric-grid operations-hero-metrics"><Metric label="Posture" value={data().server.posture} detail={`vak ${data().server.version}`} tone={tone(data().server.posture)} /><Metric label="Uptime" value={duration(data().server.uptime_secs)} detail={`PID ${data().server.pid}`} /><Metric label="Live work" value={data().runs.length} detail={`${data().gateway.approvals.pending} approval gates`} /><Metric label="Delivery" value={data().effects.waiting} detail={`${data().effects.unknown} not sure, ${data().effects.failed} didn't send`} tone={data().effects.unknown || data().effects.failed ? "bad" : undefined} /><Metric label="Voice sessions" value={data().health.voice ? `${data().health.voice?.active_sessions ?? 0}/${data().health.voice?.max_concurrent ?? 0}` : "—"} detail={data().health.voice ? `${data().health.voice?.capacity_remaining ?? 0} capacity remaining` : "Unavailable"} tone={data().health.voice?.capacity_remaining === 0 ? "warn" : undefined} /></div>
          <Show when={data().health.voice && !data().health.voice?.historical?.available}><div class="panel error-state" role="status"><strong>Historical voice telemetry unavailable</strong><p>{data().health.voice?.historical?.reason ?? "No persisted voice aggregates are available."}</p></div></Show>
          <Show when={detailBinding()}><BindingDetail data={data()} target={detailBinding()!} /></Show>
          <Show when={detailIncident()}><IncidentDetail data={data()} incidentId={detailIncident()!} onAskDoctor={() => void askDoctor()} /></Show>
          <Show when={!hasDetail()}>
            <Show when={section() === "overview"}><Topology data={data()} selected={selected()} onSelect={setSelected} /><div class="operations-overview-grid"><WorkView data={data()} /><RuntimeView data={data()} act={act} acting={acting()} /></div></Show>
            <Show when={section() === "work"}><WorkView data={data()} /></Show>
            <Show when={section() === "runtime"}><RuntimeView data={data()} act={act} acting={acting()} /></Show>
            <Show when={section() === "channels"}><ChannelsView data={data()} refresh={() => void refetch()} /></Show>
            <Show when={section() === "automations"}><AutomationsView data={data()} /></Show>
            <Show when={section() === "providers"}><ProvidersView data={data()} /></Show>
            <Show when={section() === "incidents"}><IncidentsView data={data()} /></Show>
            <Show when={section() === "sandbox"}><SandboxView data={data()} refresh={() => void refetch()} /></Show>
          </Show>
        </>}</Show>
      </Show>
    </Show>
    <Show when={doctorOpen()}><div class="ops-dialog-backdrop" role="presentation" onClick={(event) => { if (event.target === event.currentTarget) setDoctorOpen(false); }}><section class="ops-dialog" role="dialog" aria-modal="true" aria-labelledby="doctor-title" use:trapFocus><div class="panel-title-row"><div><span class="eyebrow">Contextual diagnosis</span><h2 id="doctor-title">Ask doctor</h2><p class="dim">Scope: {workspace() === "all" ? "all workspaces" : workspace()} · {timeWindowLabels[timeWindow()]} · {operationPath()}</p></div><button type="button" class="ghost small" onClick={() => setDoctorOpen(false)} aria-label="Close doctor">Close</button></div><Show when={!doctorLoading()} fallback={<div class="empty">Collecting health checks and ledger evidence…</div>}><Show when={doctorResult()}>{(result) => <><Show when={result().failures != null}><Metric label="Failed checks" value={result().failures!} tone={result().failures ? "bad" : "good"} /></Show><Show when={result().checks?.length}><div class="ops-check-list"><For each={result().checks}>{(check) => <div class="ops-check-row"><StatusMark value={check.ok ? "pass" : "fail"} /><strong>{check.label}</strong><span>{check.detail}</span></div>}</For></div></Show><pre class="mono report-pre">{result().report}</pre></>}</Show></Show></section></div></Show>
  </div>;
}
