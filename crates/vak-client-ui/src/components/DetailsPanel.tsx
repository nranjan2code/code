import { createEffect, createMemo, createResource, createSignal, For, Show, Suspense, lazy, on, onCleanup } from "solid-js";
import { activeId, candidateReviewRequest, connection, detailsFocusOf, historyPages, isRunning, isStopping, itemsOf, lastSubmittedPrompt, presentationErrorOf, presentationOf, requestedArtifact, setDetailsFocusFor, setDockTab, setWorkbenchInspection, setActiveExecutionId, setWorkbenchTab, workbenchExecutions, workbenchInspection, workbenchLoadError, type DetailsView } from "../store";
import { answerLinks, detailTurns, turnFiles, turnStatus, type DetailTurn } from "../turnDetails";
import * as api from "../api";
import { activate, focusTopicTurn, loadHistoryPage, showLatestHistory, stopRun } from "../App";
import ActivitySteps from "./ActivitySteps";
import { ResultCard } from "./PresentationRenderer";
import Icon from "./Icon";
import Skeleton from "./Skeleton";
const ExecutionInspector = lazy(() => import("./WorkbenchPanel"));
const ALL = "conversation-history";

function Evidence(props: { turn: DetailTurn; sessionId: string }) {
  const [records, { refetch }] = createResource(() => `${props.sessionId}:${props.turn.id}:${props.turn.live}:${props.turn.steps.filter(step => step.state !== "running").length}`, async () => {
    try { return { ok: true, list: await api.listSessionSandboxRecords(props.sessionId) }; }
    catch { return { ok: false, list: null }; }
  });
  const links = createMemo(() => answerLinks(props.turn.items));
  const outcomes = createMemo(() => [...new Map(props.turn.items.flatMap(item => item.outcome ? [[item.outcome.result_id, item.outcome] as const] : [])).values()]);
  const checks = createMemo(() => {
    const candidates = (records()?.list?.records ?? []).flatMap(row => row.kind === "Candidate" && (row.record.turn_id === props.turn.id || props.turn.executionIds.includes(row.record.execution_id)) ? [row.record] : []);
    const ids = new Set(candidates.map(candidate => candidate.candidate.candidate_id));
    const rows: Array<{ key: string; label: string; status: string; evidence: string; scope: string; kind?: string }> = [];
    for (const candidate of candidates) for (const [index, check] of (candidate.draft_checks ?? []).entries()) rows.push({ key: `${candidate.record_id}:${index}`, label: `File check · ${check.path.split(/[\\/]/).pop()}`, kind: check.verifier, status: check.status, evidence: check.evidence, scope: "Draft check" });
    for (const row of records()?.list?.records ?? []) {
      if (row.kind === "WorkspaceCheck" && ids.has(row.record.candidate_id)) rows.push({ key: row.record.record_id, label: row.record.check.label, status: row.record.status, evidence: row.record.evidence, scope: "Check after applying" });
      if (row.kind === "Promotion" && ids.has(row.record.candidate_id)) for (const [index, check] of (row.record.receipt.integration?.target_checks ?? []).entries()) rows.push({ key: `${row.record.record_id}:${index}`, label: `File check · ${check.path.split(/[\\/]/).pop()}`, kind: check.verifier, status: check.status, evidence: check.evidence, scope: "Applied version check" });
    }
    return rows;
  });
  return <div class="details-evidence">
    <h3>Outcome record</h3>
    <Show when={outcomes().length} fallback={<p class="details-empty">{props.turn.live ? "The outcome has not been recorded yet." : "No outcome verification was recorded for this request."}</p>}>
      <For each={outcomes()}>{outcome => <div class="details-evidence-row"><strong>{outcome.status === "succeeded" ? "Requested outcome recorded as complete" : outcome.status === "partial" ? "Partial result" : outcome.status === "cancelled" ? "Stopped" : outcome.status === "failed" || outcome.status === "denied" ? "Could not complete" : "Outcome pending"}</strong>
        <Show when={outcome.evidence_state}><p>Evidence: {outcome.evidence_state?.replace(/_/g, " ")}</p></Show>
        <Show when={outcome.human_review}><p>Review: {outcome.human_review?.replace(/_/g, " ")}</p></Show>
        <details><summary>Recorded identifiers</summary><pre>{JSON.stringify(outcome, null, 2)}</pre></details>
      </div>}</For>
    </Show>
    <h3>Recorded checks</h3>
    <p class="details-hint">Each check applies to the draft or applied version it inspected. A finished command alone does not verify the answer.</p>
    <Show when={(records()?.ok === false)}><div class="details-problem" role="alert">Couldn’t load check records. <button type="button" class="btn" onClick={() => void refetch()}>Try again</button></div></Show>
    <Show when={!(records()?.ok === false) && !records.loading && checks().length === 0}><p class="details-empty">No file checks were recorded for this request.</p></Show>
    <Show when={records.loading}><p role="status">Loading check records…</p></Show>
    <For each={checks()}>{check => <div class="details-evidence-row" data-check-status={check.status}><strong><Icon name={check.status === "passed" ? "check" : "warning"} size={14} /> {check.label}</strong><p>{check.scope} · {check.status.replace(/_/g, " ")}</p><details open={check.status === "failed"}><summary>Check result</summary><Show when={check.kind}><p>Check type: {check.kind}</p></Show><pre>{check.evidence}</pre></details></div>}</For>
    <h3>Links in this answer</h3>
    <p class="details-hint">Links are references, not confirmation that every claim was checked.</p>
    <Show when={links().length} fallback={<p class="details-empty">No source links were recorded in this answer.</p>}><ul class="details-source-list"><For each={links()}>{link => <li><a href={link.url} target="_blank" rel="noopener noreferrer">{link.label}</a></li>}</For></ul></Show>
  </div>;
}

export default function DetailsPanel() {
  const sid = () => activeId();
  const focus = () => sid() ? detailsFocusOf(sid()!) : { turnId: null, view: "activity" as const };
  const turns = createMemo(() => detailTurns(presentationOf(sid()), itemsOf(sid()), isRunning(sid()) && !historyPages[sid() ?? ""]?.newer, workbenchExecutions()));
  const selected = () => focus().turnId === ALL ? undefined : focus().turnId ? turns().find(turn => turn.id === focus().turnId) : turns().find(turn => turn.live) ?? turns()[turns().length - 1];
  const [inspector, setInspector] = createSignal(false);
  const [inspectIds, setInspectIds] = createSignal<string[] | undefined>();
  const [atTail, setAtTail] = createSignal(true);
  let scrollBody: HTMLDivElement | undefined;
  const jumpToLatest = () => { if (scrollBody) scrollBody.scrollTop = scrollBody.scrollHeight; setAtTail(true); };
  createEffect(on(() => `${sid()}:${focus().turnId}:${focus().view}`, () => setAtTail(true)));
  createEffect(() => {
    const turn = selected();
    // Follow only while the reader is already at the end. Historical turns
    // and a person inspecting an earlier step never jump on a new event.
    const revision = turn?.steps.map(step => `${step.id}:${step.state}:${step.result?.length ?? 0}`).join("|");
    if (!revision || focus().turnId || focus().view !== "activity" || !turn?.live || !atTail()) return;
    const frame = requestAnimationFrame(() => { if (scrollBody?.isConnected && !focus().turnId && atTail()) jumpToLatest(); });
    onCleanup(() => cancelAnimationFrame(frame));
  });
  const setView = (view: DetailsView) => { const session = sid(); if (session) setDetailsFocusFor(session, focus().turnId, view); };
  const choose = (id: string | null) => { const session = sid(); if (session) setDetailsFocusFor(session, id, focus().view); setInspector(false); };
  createEffect(on(sid, () => { setInspector(false); setInspectIds(undefined); }));
  createEffect(on(lastSubmittedPrompt, prompt => { if (prompt?.sessionId === sid()) { choose(null); setView("activity"); } }, { defer: true }));
  const inspect = (id: string) => { setActiveExecutionId(id); setWorkbenchTab("execution"); setInspectIds(selected()?.executionIds ?? [id]); setInspector(true); };
  createEffect(() => {
    const request = workbenchInspection();
    if (request?.sessionId !== sid()) return;
    const turn = turns().find(turn => request.executionId && turn.executionIds.includes(request.executionId));
    if (turn && sid()) setDetailsFocusFor(sid()!, turn.id, "activity");
    setInspectIds(turn?.executionIds); setInspector(true); setWorkbenchInspection(null);
  });
  createEffect(() => {
    const request = candidateReviewRequest();
    if (!request) return;
    if (request.sessionId && request.sessionId !== sid()) { void activate(request.sessionId); return; }
    const turn = turns().find(turn => turn.executionIds.includes(request.executionId));
    if (turn && sid()) setDetailsFocusFor(sid()!, turn.id, "files");
    setInspectIds(turn?.executionIds); setInspector(true);
  });
  createEffect(on(requestedArtifact, path => { if (path) { setInspectIds(undefined); setInspector(true); } }));
  const follow = async () => { choose(null); setView("activity"); if (sid() && historyPages[sid()!]?.newer) await showLatestHistory(sid()!); };
  const showInConversation = () => {
    const turn = selected();
    if (!turn) return;
    const node = [...document.querySelectorAll<HTMLElement>(".semantic-turn[data-turn]")].find(node => node.dataset.turn === turn.id);
    if (node) { if (matchMedia("(max-width: 900px)").matches) setDockTab(null); node.scrollIntoView({ block: "center", behavior: "auto" }); }
  };
  const error = () => presentationErrorOf(sid()) || workbenchLoadError();
  return <div class="details-panel">
    <Show when={sid()} fallback={<p class="details-empty">Open a conversation to see its details.</p>}>
      <Show when={!inspector()}>
      <div class="details-selector"><label for="details-request">Request</label><select id="details-request" onChange={event => { const value = event.currentTarget.value; if (value === "follow-current") void follow(); else choose(value); }}>
        <option value="follow-current" selected={!focus().turnId}>Follow current work</option>
        <For each={[...turns()].reverse()}>{turn => <option value={turn.id} selected={focus().turnId === turn.id}>{turn.live ? "Now" : "Earlier"} · {turn.request || "Recorded activity"}</option>}</For>
        <Show when={focus().turnId && focus().turnId !== ALL && !selected()}><option value={focus().turnId!} selected>Selected request · outside loaded history</option></Show>
        <option value={ALL} selected={focus().turnId === ALL}>Conversation history</option>
      </select></div>
      <Show when={focus().turnId && isRunning(sid())}><div class="details-follow" role="status"><span><span class="activity-dot" />Work is running</span><button type="button" class="btn" onClick={() => void follow()}>Follow live</button></div></Show>
      </Show>
      <Show when={inspector()} fallback={<>
        <Show when={focus().turnId !== ALL}>
          <div class="details-tabs" role="group" aria-label="Detail views"><For each={[["activity", "Activity"], ["files", "Files"], ["evidence", "Evidence"]] as const}>{([view, label]) => <button type="button" aria-pressed={focus().view === view} onClick={() => setView(view)}>{label}<Show when={view === "files" && selected() && turnFiles(selected()!).length}> ({turnFiles(selected()!).length})</Show></button>}</For></div>
        </Show>
        <div class="details-scroll" ref={scrollBody} onScroll={event => { const node = event.currentTarget; setAtTail(node.scrollHeight - node.scrollTop - node.clientHeight < 48); }}>
          <Show when={error()}><div class="details-problem" role="alert">Some activity could not be loaded. <button type="button" class="btn" onClick={() => void showLatestHistory(sid()!)}>Try again</button></div></Show>
          <Show when={focus().turnId === ALL} fallback={<Show when={selected()} fallback={<div class="details-empty"><p>{focus().turnId ? "This request is outside the loaded history." : "Activity will appear when work begins."}</p><Show when={focus().turnId}><button class="btn" type="button" onClick={() => void focusTopicTurn(sid()!, focus().turnId!)}>Load this request</button></Show></div>}>{turn => <>
            <header class="details-overview"><p class="details-turn-status" classList={{ live: turn().live || turn().steps.some(s => s.state === "waiting") }}><Show when={turn().live || turn().steps.some(s => s.state === "waiting")}><span class="activity-dot" /></Show>{turnStatus(turn())}</p><h3>{turn().request || "Recorded activity"}</h3><Show when={turn().at && Number.isFinite(Date.parse(turn().at))}><p class="details-hint">{new Date(turn().at).toLocaleString()}</p></Show><div class="details-overview-actions"><button type="button" class="details-text-action" onClick={showInConversation}>Show in conversation</button><Show when={turn().live}><button type="button" class="btn" disabled={isStopping(sid())} onClick={stopRun}>{isStopping(sid()) ? "Stopping…" : "Stop current work"}</button></Show></div></header>
            <Show when={focus().view === "activity"}><ActivitySteps turn={turn()} onInspect={inspect} /><Show when={turn().steps.some(s => s.state === "waiting")}><p class="details-attention"><Icon name="warning" size={14} />Answer the pending request in the conversation to continue.</p></Show></Show>
            <Show when={focus().view === "files"}><p class="details-hint">Files produced by this request. File status is read from the Library now.</p><Show when={turnFiles(turn()).length} fallback={<p class="details-empty">No files were recorded for this request.</p>}><div class="details-files"><For each={turnFiles(turn())}>{file => <ResultCard item={file} sessionId={sid()!} resultId={file.outcome?.result_id} currentStateOnly />}</For></div></Show></Show>
            <Show when={focus().view === "evidence"}><Evidence turn={turn()} sessionId={sid()!} /></Show>
          </>}</Show>}>
            <h3>Conversation history</h3><p class="details-hint">Loaded requests · choose one to inspect its activity, files and evidence.</p><ol class="details-history"><For each={[...turns()].reverse()}>{turn => <li><button type="button" onClick={() => choose(turn.id)}><strong>{turn.request || "Recorded activity"}</strong><span>{turnStatus(turn)} · {turnFiles(turn).length} file{turnFiles(turn).length === 1 ? "" : "s"}</span></button></li>}</For></ol>
            <Show when={sid() && historyPages[sid()!]?.older}><button type="button" class="btn" onClick={() => void loadHistoryPage(sid()!, "older")}>Load earlier requests</button></Show>
            <Show when={workbenchExecutions().some(exec => !turns().some(turn => turn.executionIds.includes(exec.id)))}><details class="details-unlinked"><summary>Commands without a recorded turn link</summary><p class="details-hint">These commands have no link to a loaded request.</p><button type="button" class="btn" onClick={() => { setInspectIds(workbenchExecutions().filter(exec => !turns().some(turn => turn.executionIds.includes(exec.id))).map(exec => exec.id)); setInspector(true); }}>Inspect commands</button></details></Show>
          </Show>
        </div>
      </>}>
        <div class="details-inspector-back"><button type="button" class="btn" onClick={() => setInspector(false)}>← Back to request details</button><span>Command details</span></div><Suspense fallback={<Skeleton kind="text" label="Loading command details" />}><ExecutionInspector executionIds={inspectIds()} /></Suspense>
      </Show>
      <Show when={!inspector() && !focus().turnId && selected()?.live && !atTail()}><button type="button" class="details-jump" onClick={jumpToLatest}>Jump to latest activity ↓</button></Show>
      <footer class="details-footer" role="status">{connection() !== "live" ? connection() === "offline" ? "Offline · activity may be out of date" : "Reconnecting · catching up on activity" : inspector() ? "Command and file tools · this conversation" : focus().turnId === ALL ? "This conversation · recorded requests" : focus().turnId ? "Request pinned · new work will not replace this view" : "Following current work"}</footer>
    </Show>
  </div>;
}
