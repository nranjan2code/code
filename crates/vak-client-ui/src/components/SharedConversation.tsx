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
  let refreshTimer: ReturnType<typeof setInterval> | undefined;

  const stop = () => {
    if (refreshTimer) clearInterval(refreshTimer);
    refreshTimer = undefined;
    setCredential(null);
    setMessages([]);
    setCandidates([]);
    setUpdatedAt(null);
    setVisibleCount(40);
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
      setUpdatedAt(new Date());
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
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
      <Show when={candidates().length > 0}><section class="shared-drafts"><h2>Saved drafts</h2><For each={candidates()}>{(record) => <div class="shared-draft"><strong>Draft {record.record.candidate?.candidate_id.slice(0, 8)}</strong><span>{record.record.candidate?.files.length ?? 0} files</span><ul><For each={record.record.candidate?.files ?? []}>{(file) => <li><Icon name="file" size={13} />{file.path}</li>}</For></ul></div>}</For></section></Show>
    </div>}>
      <div class="shared-entry"><h1>Open a shared conversation</h1><p>Paste the private invitation code you received. The code stays in this tab while you are here.</p><form onSubmit={(event) => void open(event)}><label for="shared-code">Invitation code</label><input id="shared-code" type="password" autocomplete="off" spellcheck={false} value={code()} onInput={(event) => setCode(event.currentTarget.value)} required /><button type="submit" class="btn primary" disabled={!code().trim()}>Open conversation</button></form><Show when={error()}>{(message) => <p class="shared-conversation-error" role="alert">{message()}</p>}</Show></div>
    </Show>
  </main>;
}
