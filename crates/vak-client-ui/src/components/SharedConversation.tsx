import { createSignal, For, onCleanup, Show } from "solid-js";
import type { Message } from "../types";
import Icon from "./Icon";

type SharedCandidate = {
  kind: "Candidate" | "Promotion" | "Environment";
  record: {
    candidate?: { candidate_id: string; files: Array<{ path: string }> };
    result_id?: string;
  };
};
type SharedComment = { comment_id: string; actor_id: string; text: string; path?: string; line_start?: number; line_end?: number };

function visibleText(message: Message): string {
  return message.content.filter((block) => block.type === "text").map((block) => block.text).join("\n").trim();
}

export default function SharedConversation() {
  const [code, setCode] = createSignal("");
  const [credential, setCredential] = createSignal<{ conversationId: string; token: string } | null>(null);
  const [messages, setMessages] = createSignal<Message[]>([]);
  const [candidates, setCandidates] = createSignal<SharedCandidate[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [updatedAt, setUpdatedAt] = createSignal<Date | null>(null);
  const [visibleCount, setVisibleCount] = createSignal(40);
  const [openFile, setOpenFile] = createSignal<{ candidateId: string; path: string; content?: string; imageUrl?: string; kind: string } | null>(null);
  const [comments, setComments] = createSignal<SharedComment[]>([]);
  const [fileError, setFileError] = createSignal<string | null>(null);
  let refreshTimer: ReturnType<typeof setInterval> | undefined;

  const stop = () => {
    if (refreshTimer) clearInterval(refreshTimer);
    refreshTimer = undefined;
    setCredential(null);
    setMessages([]);
    setCandidates([]);
    setUpdatedAt(null);
    setVisibleCount(40);
    if (openFile()?.imageUrl) URL.revokeObjectURL(openFile()!.imageUrl!);
    setOpenFile(null);
    setComments([]);
    setFileError(null);
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
    if (!response.ok) throw new Error(`Could not load the shared conversation (${response.status}).`);
    return response.json();
  };

  const refresh = async () => {
    const current = credential();
    if (!current || loading()) return;
    setLoading(true);
    try {
      const [transcript, records] = await Promise.all([
        read(current.conversationId, current.token, "/transcript"),
        read(current.conversationId, current.token, "/sandbox/records"),
      ]);
      if (credential()?.token !== current.token) return;
      setMessages((transcript.messages ?? []).filter((message: Message) =>
        message.role.toLowerCase() === "user" || message.role.toLowerCase() === "assistant"
      ));
      setCandidates((records.records ?? []).filter((item: SharedCandidate) => item.kind === "Candidate"));
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

  const showFile = async (candidateId: string, path: string) => {
    const current = credential();
    if (!current) return;
    setFileError(null);
    if (openFile()?.imageUrl) URL.revokeObjectURL(openFile()!.imageUrl!);
    setOpenFile(null);
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
    await refresh();
    if (credential()) refreshTimer = setInterval(() => void refresh(), 10_000);
  };

  return <main class="shared-conversation">
    <header class="shared-conversation-head"><span class="shared-brand">vak</span><span>Shared conversation</span><Show when={credential()}><button type="button" class="btn" onClick={stop}>Leave</button></Show></header>
    <Show when={!credential()} fallback={<div class="shared-conversation-content">
      <div class="shared-conversation-intro"><h1>Conversation and drafts</h1><p>You are reading a conversation shared with you. You can follow its saved results and drafts; changes and Agent requests remain with the owner.</p><Show when={updatedAt()}>{(time) => <span>Updated {time().toLocaleTimeString()}</span>}</Show></div>
      <Show when={error()}>{(message) => <p class="shared-conversation-error" role="alert">{message()}</p>}</Show>
      <section class="shared-messages" aria-label="Conversation">
        <Show when={messages().length > visibleCount()}><button type="button" class="btn" onClick={() => setVisibleCount(visibleCount() + 40)}>Show earlier messages</button></Show>
        <For each={messages().slice(-visibleCount())} fallback={<p class="shared-empty">No visible messages yet.</p>}>
          {(message) => <Show when={visibleText(message)}>{(text) => <article class="shared-message"><span class="shared-message-author">{message.role.toLowerCase() === "assistant" ? "Agent" : "Person"}</span><p>{text()}</p></article>}</Show>}
        </For>
      </section>
      <Show when={candidates().length > 0}><section class="shared-drafts"><h2>Saved drafts</h2><For each={candidates()}>{(record) => <div class="shared-draft"><strong>Draft {record.record.candidate?.candidate_id.slice(0, 8)}</strong><span>{record.record.candidate?.files.length ?? 0} files</span><ul><For each={record.record.candidate?.files ?? []}>{(file) => <li><button type="button" onClick={() => void showFile(record.record.candidate!.candidate_id, file.path)}><Icon name="file" size={13} />{file.path}</button></li>}</For></ul></div>}</For></section></Show>
      <Show when={fileError()}>{(message) => <p class="shared-conversation-error" role="alert">{message()}</p>}</Show>
      <Show when={openFile()}>{(file) => <section class="shared-file"><div class="shared-file-head"><h2>{file().path}</h2><span>Saved draft {file().candidateId.slice(0, 8)}</span><button type="button" class="btn" onClick={() => { if (file().imageUrl) URL.revokeObjectURL(file().imageUrl!); setOpenFile(null); }}>Close</button></div><Show when={file().kind === "text"}><pre>{file().content}</pre></Show><Show when={file().kind === "image"}><img src={file().imageUrl} alt={file().path} /></Show><Show when={file().kind === "binary"}><p>This saved file has no inline preview in the shared view.</p></Show><div class="shared-file-comments"><h3>Comments on this draft</h3><For each={comments().filter((comment) => !comment.path || comment.path === file().path)} fallback={<p>No comments on this file yet.</p>}>{(comment) => <div class="shared-file-comment"><strong>{comment.actor_id}</strong><Show when={comment.line_start}><span>Line {comment.line_start}{comment.line_end ? `–${comment.line_end}` : ""}</span></Show><p>{comment.text}</p></div>}</For></div></section>}</Show>
    </div>}>
      <div class="shared-entry"><h1>Open a shared conversation</h1><p>Paste the private invitation code you received. The code stays in this tab while you are here.</p><form onSubmit={(event) => void open(event)}><label for="shared-code">Invitation code</label><input id="shared-code" type="password" autocomplete="off" spellcheck={false} value={code()} onInput={(event) => setCode(event.currentTarget.value)} required /><button type="submit" class="btn primary" disabled={!code().trim()}>Open conversation</button></form><Show when={error()}>{(message) => <p class="shared-conversation-error" role="alert">{message()}</p>}</Show></div>
    </Show>
  </main>;
}
