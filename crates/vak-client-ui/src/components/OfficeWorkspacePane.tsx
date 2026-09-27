import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from "solid-js";
import * as api from "../api";
import OfficeView from "./OfficeView";

type Collaborator = { principal_id: string; display_name: string; office_room_id?: string | null; office_anchor?: string | null };

export default function OfficeWorkspacePane(props: {
  source: api.OfficeSource;
  fileName: string;
  canEdit?: boolean;
  canStart?: boolean;
  collaborators?: Collaborator[];
  candidates?: Array<{ candidateId: string; label: string }>;
  focus?: string;
  onSelect?: (anchor: string | null) => void;
  onReview?: (candidateId: string) => void;
  onClose?: () => void;
}) {
  const [room, setRoom] = createSignal<api.OfficeWorkspace | null>(null);
  const [branchId, setBranchId] = createSignal("shared");
  const [branchName, setBranchName] = createSignal("");
  const [importCandidate, setImportCandidate] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [focusedAnchor, setFocusedAnchor] = createSignal<string | null>(null);
  const branch = createMemo(() => room()?.branches.find((item) => item.branch_id === branchId() && !item.archived) ?? room()?.branches.find((item) => item.shared));
  const editorSource = createMemo<api.OfficeSource>(() => ({ ...props.source, candidateId: branch()?.head_candidate_id ?? props.source.candidateId }));
  const inRoom = () => (props.collaborators ?? []).filter((person) => person.office_room_id === room()?.room_id);

  const refresh = async () => {
    if (!props.source.sessionId) return;
    try {
      const response = await api.getOfficeWorkspaces(props.source);
      const match = response.workspaces.find((item) => item.path === props.source.path);
      setRoom(match ?? null);
      if (match && !match.branches.some((item) => item.branch_id === branchId() && !item.archived)) setBranchId(match.branches.find((item) => item.shared)?.branch_id ?? "shared");
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
  };
  createEffect(() => {
    void props.source.sessionId; void props.source.path;
    void refresh();
    const timer = window.setInterval(() => {
      if (busy()) return;
      void refresh();
      const current = room();
      if (current) void api.setOfficeFocus(props.source, current.room_id, focusedAnchor(), true).catch(() => undefined);
    }, 2500);
    onCleanup(() => window.clearInterval(timer));
  });
  createEffect(on(() => room()?.room_id, (roomId) => {
    if (roomId) void api.setOfficeFocus(props.source, roomId, null, true).catch(() => undefined);
  }));

  const start = async () => {
    if (!props.source.sessionId || !props.source.candidateId || busy()) return;
    setBusy(true); setError(null);
    try {
      const next = await api.createOfficeWorkspace(props.source.sessionId, props.source.candidateId, props.source.path);
      setRoom(next); setBranchId("shared");
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setBusy(false); }
  };

  const act = async (action: unknown) => {
    const current = room();
    if (!current || busy()) return;
    setBusy(true); setError(null);
    try {
      const result = await api.mutateOfficeWorkspaceFromSource(props.source, current.room_id, action);
      const next = result.workspace ?? result.workspaces?.[0];
      if (next) setRoom(next);
      if (result.candidate) {
        const response = await api.getOfficeWorkspaces(props.source);
        setRoom(response.workspaces.find((item) => item.room_id === current.room_id) ?? next ?? current);
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      void refresh();
    } finally { setBusy(false); }
  };

  const edit = async (operation: api.OfficeEditOp) => {
    const current = room(); const selected = branch();
    if (!current || !selected || !props.canEdit) throw new Error("You do not have permission to edit this shared draft.");
    const result = await api.mutateOfficeWorkspaceFromSource(props.source, current.room_id, { action: "edit", branch_id: selected.branch_id, expected_head: selected.head_candidate_id, ops: [operation] });
    if (result.workspace) setRoom(result.workspace);
  };

  const onFocus = (anchor: string | null) => {
    setFocusedAnchor(anchor);
    props.onSelect?.(anchor);
    const current = room();
    if (current) void api.setOfficeFocus(props.source, current.room_id, anchor, true).catch(() => undefined);
  };

  const close = () => {
    const current = room();
    if (current) void api.setOfficeFocus(props.source, current.room_id, null, false).catch(() => undefined);
    props.onClose?.();
  };
  onCleanup(() => {
    const current = room();
    if (current) void api.setOfficeFocus(props.source, current.room_id, null, false).catch(() => undefined);
  });

  return <section class="office-workspace-pane" aria-label="Office workspace">
    <header class="office-workspace-head">
      <div class="office-workspace-title"><strong>{props.fileName}</strong><span>{branch()?.name ?? "Saved draft"}{busy() ? " · Saving" : " · Draft"}</span></div>
      <Show when={inRoom().length > 0}><div class="office-workspace-people" aria-label="People working in this file"><For each={inRoom()}>{(person) => <span title={person.office_anchor ? `${person.display_name} · ${person.office_anchor}` : person.display_name}>{person.display_name}{person.office_anchor ? ` · ${person.office_anchor}` : ""}</span>}</For></div></Show>
      <Show when={!!props.onReview && !!room()}><button type="button" class="btn sm" onClick={() => { const head = branch()?.head_candidate_id; if (head) props.onReview?.(head); }}>Review draft</button></Show>
      <Show when={props.onClose}><button type="button" class="btn sm" onClick={close}>Close</button></Show>
    </header>
    {/* Reading never waits on collaboration: without a shared workspace
        the file is shown read-only, and a saved version can start one. */}
    <Show when={room()} fallback={<>
      <Show when={props.canStart}><div class="office-workspace-start"><p>Start a shared workspace so invited people can read, edit, and branch from this version.</p><button type="button" class="btn sm" disabled={busy()} onClick={() => void start()}>{busy() ? "Opening…" : "Start shared workspace"}</button></div></Show>
      <Show when={error()}>{(message) => <p class="office-workspace-error" role="alert">{message()}</p>}</Show>
      <OfficeView source={props.source} fileName={props.fileName} focus={props.focus} onSelect={props.onSelect} />
    </>}>
      {(current) => <>
        <nav class="office-workspace-toolbar" aria-label="Shared draft controls">
          <label>Version <select value={branch()?.branch_id ?? "shared"} onChange={(event) => setBranchId(event.currentTarget.value)}><For each={current().branches.filter((item) => !item.archived)}>{(item) => <option value={item.branch_id}>{item.name}{item.shared ? " · shared" : ""}</option>}</For></select></label>
          <Show when={props.canEdit}><form onSubmit={(event) => { event.preventDefault(); const name = branchName().trim(); if (!name) return; void act({ action: "branch", name, expected_head: branch()?.head_candidate_id }); setBranchName(""); }}><input aria-label="New branch name" value={branchName()} onInput={(event) => setBranchName(event.currentTarget.value)} placeholder="New version name" maxLength={80} /><button class="btn sm" type="submit" disabled={busy() || !branchName().trim()}>New branch</button></form></Show>
          <Show when={props.canEdit && !branch()?.shared && branch()?.head_candidate_id !== current().branches.find((item) => item.shared)?.head_candidate_id}><button type="button" class="btn sm" disabled={busy()} onClick={() => void act({ action: "merge", branch_id: branch()?.branch_id, expected_shared_head: current().branches.find((item) => item.shared)?.head_candidate_id })}>Merge into shared draft</button></Show>
          <Show when={props.canEdit && (props.candidates?.length ?? 0) > 0}><form onSubmit={(event) => { event.preventDefault(); if (importCandidate()) void act({ action: "import", branch_id: branch()?.branch_id, expected_head: branch()?.head_candidate_id, candidate_id: importCandidate() }); }}><select aria-label="Agent version to add" value={importCandidate()} onChange={(event) => setImportCandidate(event.currentTarget.value)}><option value="">Agent versions</option><For each={props.candidates}>{(candidate) => <option value={candidate.candidateId}>{candidate.label}</option>}</For></select><button type="submit" class="btn sm" disabled={busy() || !importCandidate()}>Add version</button></form></Show>
        </nav>
        <Show when={error()}>{(message) => <p class="office-workspace-error" role="alert">{message()}</p>}</Show>
        <OfficeView source={editorSource()} fileName={props.fileName} focus={props.focus} canEdit={props.canEdit} onEdit={edit} onSelect={onFocus} />
        <footer class="office-workspace-history"><span>{current().revisions.length} saved {current().revisions.length === 1 ? "change" : "changes"}</span><span>Accepting this draft stays with the owner in Review.</span></footer>
      </>}
    </Show>
  </section>;
}
