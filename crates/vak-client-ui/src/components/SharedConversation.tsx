import { createSignal, For, onCleanup, Show } from "solid-js";
import type { Message, OutputItem, OutputTimeline } from "../types";
import Icon from "./Icon";
import { AdaptiveTreeView, StructuredView } from "./PresentationRenderer";
import AgentMark from "./AgentMark";
import OfficeWorkspacePane from "./OfficeWorkspacePane";
import { isDocumentPath } from "../officeFiles";

type SharedCandidate = {
  kind: "Candidate" | "Promotion" | "Environment";
  record: {
    parent_candidate_id?: string | null;
    candidate?: { candidate_id: string; parent_candidate_id?: string | null; files: Array<{ path: string }> };
    result_id?: string;
  };
};
type SharedComment = { comment_id: string; actor_id: string; actor_name?: string; text: string; path?: string; anchor?: string; line_start?: number; line_end?: number };
type SharedMessage = Message & { author_id?: string; author_name?: string };
type PresentParticipant = { principal_id: string; display_name: string; office_room_id?: string | null; office_anchor?: string | null };
type SharedApproval = { request_id: string; tool: string; args_json: string; reason: string; requested_at: string };

function visibleText(message: Message): string {
  const text = message.content.filter((block) => block.type === "text").map((block) => block.text).join("\n").trim();
  const shared = message as SharedMessage;
  const prefix = shared.author_name ? `${shared.author_name}: ` : "";
  return prefix && text.startsWith(prefix) ? text.slice(prefix.length) : text;
}

export default function SharedConversation() {
  const [code, setCode] = createSignal("");
  const [credential, setCredential] = createSignal<{ conversationId: string; token: string } | null>(null);
  const [messages, setMessages] = createSignal<SharedMessage[]>([]);
  const [candidates, setCandidates] = createSignal<SharedCandidate[]>([]);
  const [sharedResults, setSharedResults] = createSignal<OutputItem[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [updatedAt, setUpdatedAt] = createSignal<Date | null>(null);
  const [visibleCount, setVisibleCount] = createSignal(40);
  const [openFile, setOpenFile] = createSignal<{ candidateId: string; path: string; content?: string; imageUrl?: string; kind: string } | null>(null);
  const [comments, setComments] = createSignal<SharedComment[]>([]);
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

  const stop = () => {
    if (refreshTimer) clearInterval(refreshTimer);
    refreshTimer = undefined;
    updatesAbort?.abort();
    updatesAbort = undefined;
    setCredential(null);
    setMessages([]);
    setCandidates([]);
    setSharedResults([]);
    setUpdatedAt(null);
    setVisibleCount(40);
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

  const refresh = async () => {
    const current = credential();
    if (!current || loading()) return;
    setLoading(true);
    try {
      const [transcript, records, presentation, pending] = await Promise.all([
        read(current.conversationId, current.token, "/transcript"),
        read(current.conversationId, current.token, "/sandbox/records"),
        read(current.conversationId, current.token, "/presentation"),
        read(current.conversationId, current.token, "/coworking/approvals"),
      ]);
      if (credential()?.token !== current.token) return;
      if (transcript) setMessages((transcript.messages ?? []).map((message: Message, index: number) => ({
        ...message,
        author_id: transcript.entries?.[index]?.author_id,
        author_name: transcript.entries?.[index]?.author_name,
      })).filter((message: Message) => message.role.toLowerCase() === "user" || message.role.toLowerCase() === "assistant"));
      setCandidates((records.records ?? []).filter((item: SharedCandidate) => item.kind === "Candidate"));
      setSharedResults(((presentation as OutputTimeline).items ?? []).filter((item) =>
        item.status !== "running" && (item.content.type === "structured" || item.content.type === "adaptive")
      ));
      setApprovals(pending.approvals ?? []);
      const selected = openFile();
      if (selected) {
        const history = await read(current.conversationId, current.token, `/sandbox/candidates/${encodeURIComponent(selected.candidateId)}/comments`);
        if (credential()?.token !== current.token) return;
        setComments(history.comments ?? []);
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
      try { const history = await read(current.conversationId, current.token, `/sandbox/candidates/${encodeURIComponent(candidateId)}/comments`); if (credential()?.token === current.token) setComments(history.comments ?? []); }
      catch (cause) { setFileError(cause instanceof Error ? cause.message : String(cause)); }
      return;
    }
    const base = `/sandbox/candidates/${encodeURIComponent(candidateId)}`;
    try {
      const [file, history] = await Promise.all([
        read(current.conversationId, current.token, `${base}/files?path=${encodeURIComponent(path)}`),
        read(current.conversationId, current.token, `${base}/comments`),
      ]);
      if (credential()?.token !== current.token) return;
      setComments(history.comments ?? []);
      if (file.kind === "text") {
        setOpenFile({ candidateId, path, content: file.content ?? "", kind: "text" });
      } else if (/\.(png|jpe?g|gif|webp|svg)$/i.test(path)) {
        const response = await fetch(`/sessions/${encodeURIComponent(current.conversationId)}${base}/files/raw?path=${encodeURIComponent(path)}`, {
          headers: { Authorization: `Bearer ${current.token}` }, credentials: "omit", cache: "no-store", referrerPolicy: "no-referrer",
        });
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
      const response = await fetch(`/sessions/${encodeURIComponent(current.conversationId)}/sandbox/candidates/${encodeURIComponent(file.candidateId)}/comments`, {
        method: "POST",
        headers: { Authorization: `Bearer ${current.token}`, "Content-Type": "application/json" },
        credentials: "omit", cache: "no-store", referrerPolicy: "no-referrer",
        body: JSON.stringify({ text: commentText().trim(), path: file.path, ...(file.kind === "office" && commentAnchor() ? { anchor: commentAnchor() } : { line_start: line }) }),
      });
      if (response.status === 401 || response.status === 403) { stop(); throw new Error("Access to this invitation ended or commenting is not allowed."); }
      if (!response.ok) throw new Error(`Could not save comment (${response.status}).`);
      const history = await read(current.conversationId, current.token, `/sandbox/candidates/${encodeURIComponent(file.candidateId)}/comments`);
      if (credential()?.token !== current.token) return;
      setComments(history.comments ?? []);
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
      <Show when={approvals().length > 0}><section class="shared-approvals" aria-label="Decisions requested"><h2>Decision needed</h2><For each={approvals()}>{(approval) => <article class="shared-approval"><strong>{agent().name} wants to use {approval.tool}</strong><p>{approval.reason || "This action needs a person’s decision before work can continue."}</p><details><summary>Review exact request</summary><pre>{approvalArgs(approval)}</pre></details><div><button type="button" class="btn primary" disabled={approvalBusy() === approval.request_id} onClick={() => void answerApproval(approval.request_id, true)}>Allow once</button><button type="button" class="btn danger" disabled={approvalBusy() === approval.request_id} onClick={() => void answerApproval(approval.request_id, false)}>Deny</button></div><span>This answers only this request. It cannot create a rule or accept files.</span></article>}</For></section></Show>
      <section class="shared-messages" aria-label="Conversation">
        <Show when={messages().length > visibleCount()}><button type="button" class="btn" onClick={() => setVisibleCount(visibleCount() + 40)}>Show earlier messages</button></Show>
        <For each={messages().slice(-visibleCount())} fallback={<p class="shared-empty">No visible messages yet.</p>}>
          {(message) => <Show when={visibleText(message)}>{(text) => <article class="shared-message"><span class="shared-message-author"><Show when={message.role.toLowerCase() === "assistant"} fallback={<>{message.author_name || "Owner"}</>}><AgentMark character={agent().character} motion={agent().animation} size={18} /><span>{agent().name}</span></Show></span><p>{text()}</p></article>}</Show>}
        </For>
      </section>
      <Show when={canMessage()}><form class="shared-message-composer" onSubmit={(event) => void postMessage(event)}><label for="shared-message-text">Add to the conversation</label><textarea id="shared-message-text" value={messageText()} onInput={(event) => setMessageText(event.currentTarget.value)} maxLength={32768} rows={3} placeholder={`Message ${agent().name} and the people here`} required /><div><span>Your message is included when the owner next asks the Agent to work.</span><button type="submit" class="btn primary" disabled={messageBusy() || !messageText().trim()}>{messageBusy() ? "Sending…" : "Send message"}</button></div></form></Show>
      <Show when={sharedResults().length > 0}><section class="shared-results" aria-label="Shared results"><h2>Results</h2><For each={sharedResults()}>{(item) => <article class="shared-result" data-result-id={item.outcome?.result_id ?? item.id}><Show when={item.content.type === "structured"}>{item.content.type === "structured" && <StructuredView output={item.content.output} fallback={item.fallback_text} />}</Show><Show when={item.content.type === "adaptive"}>{item.content.type === "adaptive" && <AdaptiveTreeView tree={item.content.tree} fallback={item.content.fallback_text} />}</Show></article>}</For></section></Show>
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
