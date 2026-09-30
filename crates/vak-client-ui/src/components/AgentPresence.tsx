import { createEffect, createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import { agentForSession, setAgentPickerOpen, setAgentPickerTab } from "../store";
import * as api from "../api";
import AgentMark from "./AgentMark";

export default function AgentPresence(props: { sessionId: string | null; working: boolean }) {
  const [hovered, setHovered] = createSignal(false);
  const [commitments, setCommitments] = createSignal<api.Commitment[]>([]);
  const [workProjection, setWorkProjection] = createSignal<import("../types").WorkProjection | null>(null);
  const [selected, setSelected] = createSignal<number | null>(null);
  const agentId = () => agentForSession(props.sessionId).id;
  const current = createMemo(() => commitments().find((c) =>
    c.episodes.some((episode) => episode.session_id === props.sessionId && !episode.ended_at),
  ));
  const stages = createMemo(() => {
    const live = workProjection();
    if (live?.contract.items.length) {
      const done = live.contract.items.filter((item) => ["succeeded", "skipped"].includes(live.items[item.item_id]?.status ?? ""));
      const active = live.contract.items.filter((item) => ["running", "waiting_approval", "ready_for_verification"].includes(live.items[item.item_id]?.status ?? ""));
      const waiting = live.contract.items.filter((item) => ["blocked", "failed", "cancelled", "interrupted"].includes(live.items[item.item_id]?.status ?? ""));
      const upcoming = live.contract.items.filter((item) => !["succeeded", "skipped", "running", "waiting_approval", "ready_for_verification", "blocked", "failed", "cancelled", "interrupted"].includes(live.items[item.item_id]?.status ?? ""));
      return [
        ...done.map((item) => ({ label: item.title, state: "done" as const, detail: "Done" })),
        ...(active.length
          ? [{ label: active[0].title, state: "active" as const, detail: live.items[active[0].item_id]?.status === "waiting_approval" ? "Needs your approval" : "In progress" }]
          : live.status === "active" ? [{ label: live.contract.objective, state: "active" as const, detail: "Work in progress" }] : []),
        ...active.slice(1).map((item) => ({ label: item.title, state: "next" as const, detail: "Also in progress" })),
        ...waiting.map((item) => ({ label: item.title, state: "blocked" as const, detail: live.items[item.item_id]?.blocker || live.items[item.item_id]?.status.replaceAll("_", " ") || "Waiting" })),
        ...upcoming.map((item) => ({ label: item.title, state: "next" as const, detail: "Up next" })),
      ];
    }
    const criteria = current()?.criteria ?? [];
    const done = criteria.filter((criterion) => criterion.result?.kind === "passed");
    const pending = criteria.filter((criterion) => criterion.result?.kind !== "passed");
    const tracked = [...done.map((criterion) => ({ label: criterion.statement, state: "done" as const, detail: "Done" })),
      ...(current() && props.working ? [{ label: current()!.spec.objective, state: "active" as const, detail: "Commitment in progress" }] : []),
      ...pending.map((criterion) => ({ label: criterion.statement, state: "next" as const, detail: "Up next" }))];
    if (tracked.length) return tracked;
    return props.working
      ? [{ label: workProjection()?.contract.objective || "Current task", state: "active" as const, detail: "Work in progress" }]
      : [];
  });
  const load = async () => {
    if (!props.working || !props.sessionId) return;
    const sessionId = props.sessionId;
    const id = agentId();
    const [work, portfolio] = await Promise.allSettled([
      api.work(sessionId),
      api.listCommitments(false, id),
    ]);
    if (!props.working || props.sessionId !== sessionId || agentId() !== id) return;
    setWorkProjection(work.status === "fulfilled" ? work.value : null);
    setCommitments(portfolio.status === "fulfilled" ? portfolio.value.commitments : []);
  };
  const reveal = () => {
    if (!props.working) return;
    setHovered(true);
    void load();
  };
  const close = (event: PointerEvent | FocusEvent) => {
    const target = event.currentTarget;
    const related = event.relatedTarget;
    if (target instanceof HTMLElement && !target.contains(related instanceof Node ? related : null)) {
      setHovered(false);
      setSelected(null);
    }
  };
  const point = (index: number, list = stages()) => {
    const stage = list[index];
    const doneCount = list.filter((item) => item.state === "done").length;
    const nextCount = list.filter((item) => item.state === "next" || item.state === "blocked").length;
    let angle = Math.PI / 2;
    if (stage?.state === "done") {
      const within = list.slice(0, index).filter((item) => item.state === "done").length;
      angle = Math.PI - ((within + 1) / (doneCount + 1)) * Math.PI / 2;
    } else if (stage?.state === "next" || stage?.state === "blocked") {
      const within = list.slice(0, index).filter((item) => item.state === "next" || item.state === "blocked").length;
      angle = ((nextCount - within) / (nextCount + 1)) * Math.PI / 2;
    }
    return { x: 74 + 64 * Math.cos(angle), y: 8 + 64 * Math.sin(angle) };
  };
  createEffect(() => {
    if (!props.working) { setHovered(false); setSelected(null); setWorkProjection(null); }
  });
  createEffect(() => {
    if (!props.working || !hovered() || !props.sessionId) return;
    const refresh = () => { void load(); };
    const timer = window.setInterval(refresh, 1200);
    refresh();
    onCleanup(() => { window.clearInterval(timer); });
  });
  const agent = () => agentForSession(props.sessionId);
  return (
    <div class="agent-presence" onPointerEnter={reveal} onPointerLeave={close} onFocusIn={reveal} onFocusOut={close}>
      <button type="button" class="agent-presence-switcher" aria-label={`Switch agent. Current agent: ${agent().name}`} title={`Switch agent · ${agent().name}`} onClick={() => { setAgentPickerTab("fleet"); setAgentPickerOpen(true); }}>
        <AgentMark character={agent().character} motion={agent().animation} size={40} state={props.working ? "working" : "idle"} class="agent-presence-mark" />
        <Show when={props.working}><span class="agent-presence-activity">Working</span></Show>
        <span class="agent-presence-chevron" aria-hidden="true">⌄</span>
      </button>
      <Show when={props.working && hovered() && stages().length > 0}>
        <div class="commitment-wheel-popover" role="group" aria-label="Commitment progress">
          <svg class="commitment-wheel" viewBox="0 0 148 82" aria-label="Commitments move from upcoming on the right through active at the bottom to completed on the left">
            <path class="commitment-wheel-track" d="M 138 8 A 64 64 0 0 1 10 8" />
            <For each={stages()}>{(stage, index) => {
              const position = () => point(index());
              return <circle class={`commitment-wheel-dot ${stage.state}`} cx={position().x} cy={position().y} r="4.5" tabIndex="0"
                aria-label={`${stage.label}: ${stage.detail}`}
                onPointerEnter={() => setSelected(index())} onFocus={() => setSelected(index())} onBlur={() => setSelected(null)} />;
            }}</For>
          </svg>
          <Show when={selected() !== null && stages()[selected()!]}>{(stage) => {
            const p = point(selected()!);
            return <div class="commitment-wheel-tip" style={{ left: `${Math.max(4, Math.min(98, (p.x / 148) * 100))}%` }}>
              <strong>{stage().label}</strong>
              <span>{stage().detail}</span>
            </div>;
          }}</Show>
        </div>
      </Show>
    </div>
  );
}
