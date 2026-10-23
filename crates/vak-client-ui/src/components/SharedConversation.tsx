import { createSignal, For, onCleanup, Show } from "solid-js";
import type { OutputTimeline } from "../types";
import type { FileResponse, VersionBinding } from "../api";
import Icon from "./Icon";
import PageView from "./PageView";
import AgentMark from "./AgentMark";
import OfficeWorkspacePane from "./OfficeWorkspacePane";
import { isDocumentPath } from "../officeFiles";
import { approvalSubjects, approvalTitle } from "../approvalSubject";

type SharedCandidate = {
  kind: "Candidate" | "Promotion" | "Environment";
  record: {
    parent_candidate_id?: string | null;
    candidate?: { candidate_id: string; parent_candidate_id?: string | null; files: Array<{ path: string }> };
    result_id?: string;
  };
};
type SharedComment = { comment_id: string; actor_id: string; actor_name?: string; text: string; path?: string; anchor?: string; line_start?: number; line_end?: number };
type PresentParticipant = { principal_id: string; display_name: string; office_room_id?: string | null; office_anchor?: string | null };
type SharedApproval = { request_id: string; tool: string; args_json: string; reason: string; requested_at: string };

export default function SharedConversation() {
  const [code, setCode] = createSignal("");
  const [credential, setCredential] = createSignal<{ conversationId: string; token: string } | null>(null);
  const [candidates, setCandidates] = createSignal<SharedCandidate[]>([]);
  const [presentation, setPresentation] = createSignal<OutputTimeline>({ schema_version: 2, session_id: "", items: [], diagnostics: [] });
  const [error, setError] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [updatedAt, setUpdatedAt] = createSignal<Date | null>(null);
  const [page, setPage] = createSignal<{ older?: string | null; newer?: string | null }>();
  let pageRequest: string | undefined;
  let revision: string | undefined;
  const [draftsShown, setDraftsShown] = createSignal(false);
  const [openFile, setOpenFile] = createSignal<{ candidateId: string; path: string; content?: string; imageUrl?: string; kind: string } | null>(null);
  const [comments, setComments] = createSignal<SharedComment[]>([]);
  /** Which artifact version each draft file is: where its thread is read and written. */
  const [bindings, setBindings] = createSignal<VersionBinding[]>([]);
  /** Where a draft file's version is read: its text, its bytes, its thread. */
  const versionUrl = (candidateId: string, path: string) => {
    const at = bindings().find((binding) => binding.candidate === candidateId && binding.path === path);
    return at ? `/library/${encodeURIComponent(at.artifact)}/versions/${encodeURIComponent(at.version)}` : null;
  };
  const threadUrl = (candidateId: string, path: string) => {
    const at = versionUrl(candidateId, path);
    return at ? `${at}/comments` : null;
  };
  const readThread = async (token: string, candidateId: string, path: string): Promise<SharedComment[]> => {
    const url = threadUrl(candidateId, path);
    if (!url) return [];
    const response = await fetch(url, { headers: { Authorization: `Bearer ${token}` }, credentials: "omit", cache: "no-store", referrerPolicy: "no-referrer" });
    if (response.status === 401 || response.status === 403) { stop(); throw new Error("This invitation has expired or access was revoked. Ask the owner for a new invitation."); }
    if (!response.ok) throw new Error(`Could not load the comments (${response.status}).`);
    return (await response.json()).comments ?? [];
  };
  const [fileError, setFileError] = createSignal<string | null>(null);
  const [participantName, setParticipantName] = createSignal("");
  const [participantId, setParticipantId] = createSignal("");
  const [agent, setAgent] = createSignal<{ name: string; character: string; animation: "subtle" | "expressive" | "off" }>({ name: "Vakyartha", character: "vak", animation: "subtle" });
  const [canComment, setCanComment] = createSignal(false);
  const [canMessage, setCanMessage] = createSignal(false);
  const [canEdit, setCanEdit] = createSignal(false);
  const [messageText, setMessageText] = createSignal("");
  const [messageBusy, setMessageBusy] = createSignal(false);
  const [presentParticipants, setPresentParticipants] = createSignal<PresentParticipant[]>([]);
  const [approvals, setApprovals] = createSignal<SharedApproval[]>([]);
  const [approvalBusy, setApprovalBusy] = createSignal<string | null>(null);
  const [commentText, setCommentText] = createSignal("");
  const [commentLine, setCommentLine] = createSignal("");
  const [commentAnchor, setCommentAnchor] = createSignal<string | null>(null);
  const [commentBusy, setCommentBusy] = createSignal(false);
  let refreshTimer: ReturnType<typeof setInterval> | undefined;
  let updatesAbort: AbortController | undefined;

  const resourceUrls = new Set<string>();
  const sharedResource = async (path: string, format: "text" | "raw") => {
    const current = credential();
    if (current && !bindings().some(binding => binding.path === path)) {
      const records = await read(current.conversationId, current.token, "/sandbox/records");
      if (credential()?.token !== current.token) throw new Error("Access to this invitation ended.");
      setBindings(records.artifacts ?? []);
    }
    const matches = bindings().filter(binding => binding.path === path);
    const versions = new Set(matches.map(binding => `${binding.artifact}/${binding.version}`));
    if (!current || versions.size !== 1) throw new Error("This file is not uniquely shared through this conversation.");
    const binding = matches[0];
    const response = await fetch(`/library/${encodeURIComponent(binding.artifact)}/versions/${encodeURIComponent(binding.version)}/${format}`, {
      headers: { Authorization: `Bearer ${current.token}` }, credentials: "omit", cache: "no-store", referrerPolicy: "no-referrer",
    });
    if (response.status === 401 || response.status === 403) { stop(); throw new Error("Access to this invitation ended."); }
    if (!response.ok) throw new Error("Could not read this shared file.");
    if (credential()?.token !== current.token) throw new Error("Access to this invitation ended.");
    return response;
  };
  const resourceReader = {
    readFile: async (path: string): Promise<FileResponse> => {
      const response = await sharedResource(path, "text");
      const file = await response.json();
      if (!credential()) throw new Error("Access to this invitation ended.");
      return { ...file, path, editable: false };
    },
    readFileBytes: async (path: string) => {
      const response = await sharedResource(path, "raw");
      const bytes = new Uint8Array(await response.arrayBuffer());
      if (!credential()) throw new Error("Access to this invitation ended.");
      return { bytes, mime: response.headers.get("Content-Type") ?? "application/octet-stream" };
    },
    readFileRaw: async (path: string) => {
      const response = await sharedResource(path, "raw");
      const blob = await response.blob();
      if (!credential()) throw new Error("Access to this invitation ended.");
      const url = URL.createObjectURL(blob);
      resourceUrls.add(url);
      return url;
    },
  };
  const stop = () => {
    for (const url of resourceUrls) URL.revokeObjectURL(url);
    resourceUrls.clear();
    if (refreshTimer) clearInterval(refreshTimer);
    refreshTimer = undefined;
    updatesAbort?.abort();
    updatesAbort = undefined;
    setCredential(null);
    setCandidates([]);
    setPresentation({ schema_version: 2, session_id: "", items: [], diagnostics: [] });
    setUpdatedAt(null);
    setPage(undefined); pageRequest = undefined; revision = undefined; setDraftsShown(false);
    if (openFile()?.imageUrl) URL.revokeObjectURL(openFile()!.imageUrl!);
    setOpenFile(null);
    setComments([]);
    setFileError(null);
    setParticipantName("");
    setParticipantId("");
    setAgent({ name: "Vakyartha", character: "vak", animation: "subtle" });
    setCanComment(false);
    setCanMessage(false);
    setCanEdit(false);
    setMessageText("");
    setPresentParticipants([]);
    setApprovals([]);
    setCommentText("");
    setCommentLine("");
    setCommentAnchor(null);
  };
  onCleanup(stop);

  const read = async (conversationId: string, token: string, path: string) => {
    const response = await fetch(`/sessions/${encodeURIComponent(conversationId)}${path}`, {
      headers: { Authorization: `Bearer ${token}` },
      credentials: "omit",
      cache: "no-store",
      referrerPolicy: "no-referrer",
    });
    if (response.status === 401 || response.status === 403) {
      stop();
      throw new Error("This invitation has expired or access was revoked. Ask the owner for a new invitation.");
    }
    // A turn temporarily owns the writable ledger. Preserve the last
    // transcript until the next scoped update announces settled content.
    if (path === "/transcript" && response.status === 409) return null;
    if (!response.ok) throw new Error(`Could not load the shared conversation (${response.status}).`);
    return response.json();
  };

  const refresh = async (before = pageRequest, force = false) => {
    const current = credential();
    if (!current || loading()) return;
    setLoading(true);
    try {
      const query = new URLSearchParams();
      if (before) query.set("before", before);
      if (!force && before === pageRequest && revision) query.set("since", revision);
      const [snapshot, pending] = await Promise.all([
        read(current.conversationId, current.token, `/snapshot${query.size ? `?${query}` : ""}`),
        read(current.conversationId, current.token, "/coworking/approvals"),
      ]);
      if (credential()?.token !== current.token) return;
      setApprovals(pending.approvals ?? []);
      if (snapshot.not_modified) { setUpdatedAt(new Date()); return; }

      pageRequest = before;
      revision = snapshot.revision;
      setPage(snapshot.page);
      const records = draftsShown() ? await read(current.conversationId, current.token, "/sandbox/records") : { records: [], artifacts: [] };
      if (credential()?.token !== current.token) return;
      setCandidates((records.records ?? []).filter((item: SharedCandidate) => item.kind === "Candidate"));
      setBindings(records.artifacts ?? []);
      setPresentation(snapshot.presentation);
      setApprovals(pending.approvals ?? []);
      const selected = openFile();
      if (selected) {
        const history = await readThread(current.token, selected.candidateId, selected.path);
        if (credential()?.token !== current.token) return;
        setComments(history);
      }
      setUpdatedAt(new Date());
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  };

  const watchUpdates = async (conversationId: string, token: string) => {
    const controller = new AbortController();
    updatesAbort = controller;
    while (!controller.signal.aborted && credential()?.token === token) {
      try {
        const response = await fetch(`/sessions/${encodeURIComponent(conversationId)}/coworking/updates`, {
          headers: { Authorization: `Bearer ${token}` }, credentials: "omit", cache: "no-store",
          referrerPolicy: "no-referrer", signal: controller.signal,
        });
        if (response.status === 401 || response.status === 403) {
          stop();
          setError("This invitation has expired or access was revoked. Ask the owner for a new invitation.");
          return;
        }
        // A historical conversation has no live handle; the periodic read remains available.
        if (response.status === 404) return;
        if (!response.ok || !response.body) throw new Error("Shared updates unavailable");
        const reader = response.body.getReader();
        const decoder = new TextDecoder();
        let pending = "";
        while (!controller.signal.aborted) {
          const { value, done } = await reader.read();
          if (done) break;
          pending += decoder.decode(value, { stream: true });
          const frames = pending.split(/\r?\n\r?\n/);
          pending = frames.pop() ?? "";
          for (const frame of frames) {
            if (frame.includes("event: revoked")) {
              stop();
              setError("This invitation has expired or access was revoked. Ask the owner for a new invitation.");
              return;
            }
            if (frame.includes("event: refresh")) void refresh();
            const data = frame.split(/\r?\n/).find((line) => line.startsWith("data:"))?.slice(5).trim();
            if (data) {
              try {
                const payload = JSON.parse(data);
                if (Array.isArray(payload.participants)) setPresentParticipants(payload.participants);
              } catch { /* malformed presence cannot affect conversation access */ }
            }
          }
        }
      } catch {
        if (controller.signal.aborted) return;
      }
      if (controller.signal.aborted || credential()?.token !== token) return;
      await new Promise<void>((resolve) => setTimeout(resolve, 2_000));
    }
  };

  const showFile = async (candidateId: string, path: string) => {
    const current = credential();
    if (!current) return;
    setFileError(null);
    if (openFile()?.imageUrl) URL.revokeObjectURL(openFile()!.imageUrl!);
    setOpenFile(null);
    if (isDocumentPath(path)) {
      setOpenFile({ candidateId, path, kind: "office" });
      try { const history = await readThread(current.token, candidateId, path); if (credential()?.token === current.token) setComments(history); }
      catch (cause) { setFileError(cause instanceof Error ? cause.message : String(cause)); }
      return;
    }
    const guest = { headers: { Authorization: `Bearer ${current.token}` }, credentials: "omit" as const, cache: "no-store" as const, referrerPolicy: "no-referrer" as const };
    try {
      const at = versionUrl(candidateId, path);
      if (!at) throw new Error("This file is not shared through this conversation. Ask the owner if you need it.");
      const [file, history] = await Promise.all([
        fetch(`${at}/text`, guest).then((response) => {
          if (response.status === 401 || response.status === 403) { stop(); throw new Error("Access to this invitation ended."); }
          if (!response.ok) throw new Error(`Could not open the saved file (${response.status}).`);
          return response.json();
        }),
        readThread(current.token, candidateId, path),
      ]);
      if (credential()?.token !== current.token) return;
      setComments(history);
      if (file.kind === "text") {
        setOpenFile({ candidateId, path, content: file.content ?? "", kind: "text" });
      } else if (/\.(png|jpe?g|gif|webp|svg)$/i.test(path)) {
        const response = await fetch(`${at}/raw`, guest);
        if (response.status === 401 || response.status === 403) { stop(); throw new Error("Access to this invitation ended."); }
        if (!response.ok) throw new Error(`Could not open saved image (${response.status}).`);
        const imageUrl = URL.createObjectURL(await response.blob());
        if (credential()?.token !== current.token) { URL.revokeObjectURL(imageUrl); return; }
        setOpenFile({ candidateId, path, imageUrl, kind: "image" });
      } else {
        setOpenFile({ candidateId, path, kind: "binary" });
      }
    } catch (cause) {
      setFileError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const open = async (event: SubmitEvent) => {
    event.preventDefault();
    const trimmed = code().trim();
    const separator = trimmed.indexOf(".");
    if (separator <= 0 || separator === trimmed.length - 1) {
      setError("Paste the complete invitation code from the conversation owner.");
      return;
    }
    stop();
    setError(null);
    setCredential({ conversationId: trimmed.slice(0, separator), token: trimmed.slice(separator + 1) });
    setCode("");
    try {
      const current = credential();
      if (!current) return;
      const me = await read(current.conversationId, current.token, "/coworking/me");
      if (!me.agent?.name || !me.agent?.character) throw new Error("This conversation has no complete Agent identity.");
      setParticipantName(me.display_name ?? "Guest");
      setParticipantId(me.principal_id ?? "");
      const animation = ["subtle", "expressive", "off"].includes(me.agent.animation) ? me.agent.animation as "subtle" | "expressive" | "off" : "subtle";
      setAgent({ name: me.agent.name, character: me.agent.character, animation });
      setCanComment(Array.isArray(me.capabilities) && me.capabilities.includes("comment"));
      setCanMessage(Array.isArray(me.capabilities) && me.capabilities.includes("message"));
      setCanEdit(Array.isArray(me.capabilities) && me.capabilities.includes("edit"));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      return;
    }
    await refresh();
    const current = credential();
    if (current) {
      refreshTimer = setInterval(() => void refresh(), 10_000);
      void watchUpdates(current.conversationId, current.token);
    }
  };

  const answerApproval = async (requestId: string, approve: boolean) => {
    const current = credential();
    if (!current || approvalBusy()) return;
    setApprovalBusy(requestId);
    setError(null);
    try {
      const response = await fetch(`/sessions/${encodeURIComponent(current.conversationId)}/coworking/approvals/${encodeURIComponent(requestId)}`, {
        method: "POST",
        headers: { Authorization: `Bearer ${current.token}`, "Content-Type": "application/json" },
        credentials: "omit", cache: "no-store", referrerPolicy: "no-referrer",
        body: JSON.stringify({ approve }),
      });
      if (response.status === 401 || response.status === 403) { stop(); throw new Error("Access to this decision ended."); }
      if (response.status === 404) throw new Error("This decision was already answered or expired.");
      if (!response.ok) throw new Error(`Could not answer decision (${response.status}).`);
      await refresh();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setApprovalBusy(null);
    }
  };

  const approvalArgs = (approval: SharedApproval) => {
    try { return JSON.stringify(JSON.parse(approval.args_json), null, 2); } catch { return approval.args_json; }
  };

  const postMessage = async (event: SubmitEvent) => {
    event.preventDefault();
    const current = credential();
    const text = messageText().trim();
    if (!current || !canMessage() || !text || messageBusy()) return;
    setMessageBusy(true);
    setError(null);
    try {
      const response = await fetch(`/sessions/${encodeURIComponent(current.conversationId)}/coworking/messages`, {
        method: "POST",
        headers: { Authorization: `Bearer ${current.token}`, "Content-Type": "application/json" },
        credentials: "omit", cache: "no-store", referrerPolicy: "no-referrer",
        body: JSON.stringify({ text, request_id: crypto.randomUUID() }),
      });
      if (response.status === 401 || response.status === 403) { stop(); throw new Error("Access to this invitation ended or messages are not allowed."); }
      if (response.status === 409) throw new Error("The Agent is finishing work. Send this message when the turn settles.");
      if (!response.ok) throw new Error(`Could not send message (${response.status}).`);
      setMessageText("");
      await refresh();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setMessageBusy(false);
    }
  };

  const postComment = async (event: SubmitEvent) => {
    event.preventDefault();
    const current = credential();
    const file = openFile();
    if (!current || !file || !canComment() || !commentText().trim() || commentBusy()) return;
    const line = commentLine().trim() ? Number(commentLine()) : undefined;
    if (line !== undefined && (!Number.isSafeInteger(line) || line < 1)) {
      setFileError("Enter a positive line number, or leave the line blank for a whole-file comment.");
      return;
    }
    setCommentBusy(true);
    setFileError(null);
    try {
      const thread = threadUrl(file.candidateId, file.path);
      if (!thread) throw new Error("This file is not shared through this conversation, so it cannot be commented on here.");
      const response = await fetch(thread, {
        method: "POST",
        headers: { Authorization: `Bearer ${current.token}`, "Content-Type": "application/json" },
        credentials: "omit", cache: "no-store", referrerPolicy: "no-referrer",
        body: JSON.stringify({ text: commentText().trim(), ...(file.kind === "office" && commentAnchor() ? { anchor: commentAnchor() } : { line_start: line }) }),
      });
      if (response.status === 401 || response.status === 403) { stop(); throw new Error("Access to this invitation ended or commenting is not allowed."); }
      if (!response.ok) throw new Error(`Could not save comment (${response.status}).`);
      const history = await readThread(current.token, file.candidateId, file.path);
      if (credential()?.token !== current.token) return;
      setComments(history);
      setCommentText("");
      setCommentLine("");
      setCommentAnchor(null);
    } catch (cause) {
      setFileError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setCommentBusy(false);
    }
  };

  return <main class="shared-conversation">
    <header class="shared-conversation-head"><span class="shared-brand">Vakyartha</span><span>Shared conversation</span><Show when={credential()}><span class="shared-agent-identity"><AgentMark character={agent().character} motion={agent().animation} size={22} /><span>{agent().name}</span></span><Show when={presentParticipants().filter((person) => person.principal_id !== participantId())} keyed>{(people) => <Show when={people.length > 0}><span class="shared-presence">{people.map((person) => person.display_name).join(", ")} {people.length === 1 ? "is" : "are"} here</span></Show>}</Show><button type="button" class="btn" onClick={stop}>Leave</button></Show></header>
    <Show when={!credential()} fallback={<div class="shared-conversation-content">
      <div class="shared-conversation-intro"><h1>Conversation and drafts</h1><p>{participantName() ? `${participantName()}, you can` : "You can"} follow this conversation{canMessage() ? ", add messages" : ""}, and review its saved drafts{canComment() ? " with comments" : ""}. The owner decides when the Agent works.</p><Show when={updatedAt()}>{(time) => <span>Updated {time().toLocaleTimeString()}</span>}</Show></div>
      <Show when={error()}>{(message) => <p class="shared-conversation-error" role="alert">{message()}</p>}</Show>
      <Show when={approvals().length > 0}><section class="shared-approvals" aria-label="Decisions requested"><h2>Decision needed</h2><For each={approvals()}>{(approval) => <article class="shared-approval"><strong>{agent().name} wants to use {approvalTitle(approval.tool, approval.args_json)}</strong><For each={approvalSubjects(approvalArgs(approval))}>{(subject) => <code class="ap-primary"><span class="ap-primary-key">{subject.key}</span>{subject.value}</code>}</For><p>{approval.reason || "This action needs a person’s decision before work can continue."}</p><details><summary>Review exact request</summary><pre>{approvalArgs(approval)}</pre></details><div><button type="button" class="btn primary" disabled={approvalBusy() === approval.request_id} onClick={() => void answerApproval(approval.request_id, true)}>Allow once</button><button type="button" class="btn danger" disabled={approvalBusy() === approval.request_id} onClick={() => void answerApproval(approval.request_id, false)}>Deny</button></div><span>This answers only this request. It cannot create a rule or accept files.</span></article>}</For></section></Show>
      <section class="shared-messages" aria-label="Conversation">
        <Show when={page()?.older}><button type="button" class="btn" disabled={loading()} onClick={() => void refresh(page()!.older!)}>Earlier messages</button></Show>
        <PageView page={{ format: "vak-page", title: "Shared conversation", items: presentation().items, text: "" }}
          versionId={`shared:${credential()?.conversationId}:${revision ?? ""}`} hideUser={false}
          resourceReader={resourceReader} />
      </section>
      <Show when={page()?.newer}><button type="button" class="btn" disabled={loading()} onClick={() => void refresh(page()!.newer!)}>Newer messages</button></Show>
      <Show when={canMessage()}><form class="shared-message-composer" onSubmit={(event) => void postMessage(event)}><label for="shared-message-text">Add to the conversation</label><textarea id="shared-message-text" value={messageText()} onInput={(event) => setMessageText(event.currentTarget.value)} maxLength={32768} rows={3} placeholder={`Message ${agent().name} and the people here`} required /><div><span>Your message is included when the owner next asks the Agent to work.</span><button type="submit" class="btn primary" disabled={messageBusy() || !messageText().trim()}>{messageBusy() ? "Sending…" : "Send message"}</button></div></form></Show>

      <Show when={!draftsShown()}><button type="button" class="btn" onClick={() => { setDraftsShown(true); void refresh(pageRequest, true); }}>Show saved drafts</button></Show>
      <Show when={candidates().length > 0}><section class="shared-drafts"><h2>Saved drafts</h2><For each={candidates()}>{(record) => <div class="shared-draft"><strong>Draft {record.record.candidate?.candidate_id.slice(0, 8)}</strong><span>{record.record.candidate?.files.length ?? 0} files</span><ul><For each={record.record.candidate?.files ?? []}>{(file) => <li><button type="button" onClick={() => void showFile(record.record.candidate!.candidate_id, file.path)}><Icon name="file" size={13} />{file.path}</button></li>}</For></ul></div>}</For></section></Show>
      <Show when={fileError()}>{(message) => <p class="shared-conversation-error" role="alert">{message()}</p>}</Show>
      <Show when={openFile()}>{(file) => <Show when={file().kind === "office"} fallback={<section class="shared-file"><div class="shared-file-head"><h2>{file().path}</h2><span>Saved draft {file().candidateId.slice(0, 8)}</span><button type="button" class="btn" onClick={() => { if (file().imageUrl) URL.revokeObjectURL(file().imageUrl!); setOpenFile(null); }}>Close</button></div><Show when={file().kind === "text"}><pre>{file().content}</pre></Show><Show when={file().kind === "image"}><img src={file().imageUrl} alt={file().path} /></Show><Show when={file().kind === "binary"}><p>This saved file has no inline preview in the shared view.</p></Show></section>}><section class="shared-office-overlay"><OfficeWorkspacePane
        source={{ path: file().path, sessionId: credential()?.conversationId, candidateId: file().candidateId, token: credential()?.token }}
        fileName={file().path}
        canEdit={canEdit()}
        collaborators={presentParticipants()}
        candidates={candidates()
          .filter((record) => record.record.parent_candidate_id && record.record.candidate?.candidate_id !== file().candidateId && record.record.candidate?.files.some((entry) => entry.path === file().path))
          .map((record, index) => ({ candidateId: record.record.candidate!.candidate_id, label: `Saved version ${index + 1}` }))}
        onSelect={(anchor) => setCommentAnchor(anchor)}
        onClose={() => setOpenFile(null)}
      /></section><section class="shared-office-comments"><h2>Comments on this draft</h2><Show when={canComment()}><form onSubmit={(event) => void postComment(event)}><label for="shared-office-comment">{commentAnchor() ? `Comment on ${commentAnchor()}` : "Comment on this file"}</label><textarea id="shared-office-comment" value={commentText()} onInput={(event) => setCommentText(event.currentTarget.value)} maxLength={4000} rows={2} required /><button class="btn sm" type="submit" disabled={commentBusy() || !commentText().trim()}>{commentBusy() ? "Saving…" : "Add comment"}</button></form></Show><For each={comments().filter((comment) => !comment.path || comment.path === file().path)} fallback={<p>No comments on this file yet.</p>}>{(comment) => <article><strong>{comment.actor_name ?? comment.actor_id}</strong><Show when={comment.anchor}><code>{comment.anchor}</code></Show><p>{comment.text}</p></article>}</For></section></Show>}</Show>
    </div>}>
      <div class="shared-entry"><h1>Open a shared conversation</h1><p>Paste the private invitation code you received. The code stays in this tab while you are here.</p><form onSubmit={(event) => void open(event)}><label for="shared-code">Invitation code</label><input id="shared-code" type="password" autocomplete="off" spellcheck={false} value={code()} onInput={(event) => setCode(event.currentTarget.value)} required /><button type="submit" class="btn primary" disabled={!code().trim()}>Open conversation</button></form><Show when={error()}>{(message) => <p class="shared-conversation-error" role="alert">{message()}</p>}</Show></div>
    </Show>
  </main>;
}
