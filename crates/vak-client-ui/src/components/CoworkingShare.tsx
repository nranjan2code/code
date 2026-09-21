import { createEffect, createSignal, For, Show } from "solid-js";
import * as api from "../api";
import { trapFocus } from "../focusTrap";
import Icon from "./Icon";

export default function CoworkingShare(props: { sessionId: string; onClose: () => void }) {
  const [invitations, setInvitations] = createSignal<api.CoworkingInvitation[]>([]);
  const [name, setName] = createSignal("");
  const [hours, setHours] = createSignal(24);
  const [canComment, setCanComment] = createSignal(false);
  const [canMessage, setCanMessage] = createSignal(true);
  const [issuedToken, setIssuedToken] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [copied, setCopied] = createSignal(false);

  const refresh = async () => {
    try {
      setInvitations((await api.listCoworkingInvitations(props.sessionId)).invitations);
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  };
  createEffect(() => { void props.sessionId; void refresh(); });

  const invite = async (event: SubmitEvent) => {
    event.preventDefault();
    if (!name().trim() || busy()) return;
    setBusy(true);
    setError(null);
    try {
      const response = await api.createCoworkingInvitation(props.sessionId, name().trim(), hours(), canComment(), canMessage());
      setIssuedToken(`${response.invitation.conversation_id}.${response.token}`);
      setCopied(false);
      setName("");
      await refresh();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };
  const revoke = async (grantId: string) => {
    setBusy(true);
    setError(null);
    try {
      await api.revokeCoworkingInvitation(props.sessionId, grantId);
      await refresh();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };
  const copyToken = async () => {
    const token = issuedToken();
    if (!token) return;
    try {
      await navigator.clipboard.writeText(token);
      setCopied(true);
    } catch {
      setError("Clipboard access is unavailable. Select and copy the invitation token below.");
    }
  };

  return <div class="modal-backdrop" role="presentation" onClick={props.onClose}>
    <section class="modal coworking-share" role="dialog" aria-modal="true" aria-labelledby="coworking-share-title" onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { if (event.key === "Escape") props.onClose(); }} use:trapFocus>
      <header class="modal-header">
        <div><h2 id="coworking-share-title">Invite someone to this conversation</h2><p>They can join the conversation and review its saved drafts. Their messages never start Agent work or change your workspace.</p></div>
        <button type="button" class="icon-button subtle" aria-label="Close sharing" onClick={props.onClose}><Icon name="close" /></button>
      </header>
      <Show when={error()}>{(message) => <p class="coworking-share-error" role="alert">{message()}</p>}</Show>
      <Show when={issuedToken()}>{(token) => <div class="coworking-issued" aria-live="polite">
        <strong>Invitation created</strong>
        <p>Send the private code through a channel you trust. It appears only once. The recipient can open <a href={`${api.backendUrl()}/app/?shared=1`} target="_blank" rel="noopener noreferrer">the shared conversation view</a> and paste it there.</p>
        <div class="coworking-token"><input readOnly aria-label="Invitation code" value={token()} onFocus={(event) => event.currentTarget.select()} /><button type="button" class="btn" onClick={() => void copyToken()}><Icon name={copied() ? "check" : "copy"} size={14} /> {copied() ? "Copied" : "Copy"}</button></div>
        <button type="button" class="ghost small" onClick={() => setIssuedToken(null)}>Done with token</button>
      </div>}</Show>
      <form class="coworking-invite-form" onSubmit={(event) => void invite(event)}>
        <label for="coworking-name">Person’s name</label>
        <input id="coworking-name" value={name()} onInput={(event) => setName(event.currentTarget.value)} maxLength={120} placeholder="Name for this invitation" required />
        <label for="coworking-expiry">Access expires</label>
        <select id="coworking-expiry" value={hours()} onChange={(event) => setHours(Number(event.currentTarget.value))}><option value={24}>In 1 day</option><option value={168}>In 7 days</option><option value={720}>In 30 days</option></select>
        <label class="coworking-comment-option"><input type="checkbox" checked={canComment()} onChange={(event) => setCanComment(event.currentTarget.checked)} /> Allow comments on saved drafts</label>
        <label class="coworking-comment-option"><input type="checkbox" checked={canMessage()} onChange={(event) => setCanMessage(event.currentTarget.checked)} /> Allow messages in the conversation</label>
        <button type="submit" class="btn primary" disabled={busy() || !name().trim()}>{busy() ? "Creating…" : "Create invitation"}</button>
      </form>
      <div class="coworking-invitations">
        <h3>Invitations</h3>
        <Show when={!loading()} fallback={<p class="dim">Loading invitations…</p>}>
          <For each={invitations()} fallback={<p class="dim">No one has been invited to this conversation.</p>}>
            {(invitation) => <div class="coworking-invitation">
              <div><strong>{invitation.display_name}</strong><span>{invitation.status === "active" ? `${invitation.capabilities.includes("message") ? "Conversation" : "Read"}${invitation.capabilities.includes("comment") ? ", draft comments" : ""} · until ${new Date(invitation.expires_at).toLocaleString()}` : invitation.status === "revoked" ? "Access revoked" : "Access expired"}</span></div>
              <Show when={invitation.status === "active"}><button type="button" class="btn danger" disabled={busy()} onClick={() => void revoke(invitation.grant_id)}>Revoke</button></Show>
            </div>}
          </For>
        </Show>
      </div>
    </section>
  </div>;
}
