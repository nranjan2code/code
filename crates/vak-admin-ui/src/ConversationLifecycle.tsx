/// Conversations › lifecycle (plan M7a-e, docs/design/74 §6.2 A2 and A3):
/// the trash with the day each conversation is erased, holds, erasure
/// with the title typed, the receipts erasure leaves, and one
/// conversation's place in its life. Nothing here shows what a
/// conversation said.

import { createResource, createSignal, For, Show } from "solid-js";
import { api } from "./api";
import { pushToast } from "./store";
import { confirmDestructive } from "./display";

const day = (iso?: string | null) =>
  iso ? new Date(iso).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" }) : "";
const CAUSE: Record<string, string> = { person: "A person asked", policy: "Its time in the trash ended" };
const SCOPE: Record<string, string> = { conversation: "Conversation", draft: "Draft", guest: "A guest's messages", account: "A connected account's data" };
const told = (err: unknown) => String(err instanceof Error ? err.message : err);

/** The typed confirmation: erasing needs the conversation's title. */
function EraseForm(props: { sessionId: string; title: string; onDone: () => void; onCancel: () => void }) {
  const [preview] = createResource(() => props.sessionId, (id) => api.erasurePreview(id));
  const [typed, setTyped] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const ready = () => Boolean(preview()) && !preview()!.preview.held && typed().trim() === preview()!.confirm;
  const erase = async () => {
    const shown = preview();
    if (!shown || !ready()) return;
    setBusy(true);
    try {
      await api.eraseConversation(props.sessionId, shown.preview.digest, typed().trim());
      pushToast("info", "Conversation erased. Its receipt is listed below.");
      props.onDone();
    } catch (err) {
      pushToast("alert", told(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div class="lifecycle-erase" role="group" aria-label={`Erase ${props.title}`}>
      <Show when={preview()} fallback={<p class="dim">{preview.error ? `Could not read what this would erase: ${preview.error}` : "Reading what this would erase…"}</p>}>
        {(shown) => (
          <>
            <p>
              Erasing <strong>{props.title}</strong> destroys the keys of {shown().preview.conversations.length}{" "}
              {shown().preview.conversations.length === 1 ? "conversation" : "conversations"} (it and the work it delegated),{" "}
              {shown().preview.artifacts.length} unkept {shown().preview.artifacts.length === 1 ? "draft" : "drafts"} and{" "}
              {shown().preview.memory_notes} memory {shown().preview.memory_notes === 1 ? "note" : "notes"}. It cannot be undone.
            </p>
            <p class="dim">
              Not reached: copies the AI services hold, backups made before now
              <Show when={shown().preview.sent_outside > 0}>, and {shown().preview.sent_outside} already sent outside</Show>.
            </p>
            <Show when={!shown().preview.held} fallback={<p>This conversation is on hold. Release the hold before erasing it.</p>}>
              <label class="lifecycle-confirm">
                <span>Type <strong>{shown().confirm}</strong> to confirm</span>
                <input value={typed()} autocomplete="off" spellcheck={false} onInput={(event) => setTyped(event.currentTarget.value)} />
              </label>
            </Show>
            <div class="lifecycle-actions">
              <button class="button small ghost" disabled={busy()} onClick={props.onCancel}>Cancel</button>
              <button class="button small danger" disabled={!ready() || busy()} onClick={() => void erase()}>{busy() ? "Erasing…" : "Erase for good"}</button>
            </div>
          </>
        )}
      </Show>
    </div>
  );
}

/** The trash and the receipts, under the Conversations list. */
export function ConversationTrash(props: { onChanged?: () => void }) {
  const [trash, { refetch }] = createResource(() => api.trashedSessions());
  const [receipts, { refetch: reread }] = createResource(() => api.erasureReceipts());
  const [erasing, setErasing] = createSignal("");
  const [busy, setBusy] = createSignal("");
  const changed = () => {
    void refetch();
    void reread();
    props.onChanged?.();
  };
  const act = async (id: string, work: () => Promise<unknown>, done: string) => {
    setBusy(id);
    try {
      await work();
      pushToast("info", done);
      changed();
    } catch (err) {
      pushToast("alert", told(err));
    } finally {
      setBusy("");
    }
  };
  return (
    <>
      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Trash</h2>
            <p class="dim">Hidden everywhere, search included. A conversation is erased on the day shown, unless it is restored or put on hold. The keep time is set under Data › Retention.</p>
          </div>
        </div>
        <Show when={!trash.error} fallback={<p class="dim">Could not read the trash: {`${trash.error}`}</p>}>
          <Show when={(trash()?.sessions ?? []).length > 0} fallback={<p class="dim">The trash is empty.</p>}>
            <div class="ops-table-wrap">
              <table class="ops-table">
                <thead><tr><th>Conversation</th><th>Agent</th><th>In the trash since</th><th>Erased on</th><th></th></tr></thead>
                <tbody>
                  <For each={trash()?.sessions ?? []}>
                    {(session) => (
                      <>
                        <tr>
                          <td>{session.title || "Untitled"}</td>
                          <td>{session.agent?.name ?? ""}</td>
                          <td>{day(session.trashed_at)}</td>
                          <td>{session.held ? "On hold" : day(session.erase_on)}</td>
                          <td>
                            <div class="lifecycle-actions">
                              <button class="button small ghost" disabled={busy() === session.session_id} onClick={() => void act(session.session_id, () => api.restoreSession(session.session_id), "Conversation restored")}>Restore</button>
                              <button class="button small ghost" disabled={busy() === session.session_id} onClick={() => void act(session.session_id, () => api.holdConversation(session.session_id, !session.held), session.held ? "Hold released" : "Conversation put on hold")}>{session.held ? "Release hold" : "Hold"}</button>
                              <button class="button small danger ghost" disabled={session.held || busy() === session.session_id} onClick={() => setErasing(erasing() === session.session_id ? "" : session.session_id)}>Erase…</button>
                            </div>
                          </td>
                        </tr>
                        <Show when={erasing() === session.session_id}>
                          <tr><td colSpan={5}><EraseForm sessionId={session.session_id} title={session.title || "Untitled"} onCancel={() => setErasing("")} onDone={() => { setErasing(""); changed(); }} /></td></tr>
                        </Show>
                      </>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          </Show>
        </Show>
      </section>
      <section class="panel">
        <div class="panel-title-row">
          <div>
            <h2>Erasure receipts</h2>
            <p class="dim">One signed record for each erasure. A receipt holds counts and ids, never what was erased.</p>
          </div>
        </div>
        <Show when={(receipts()?.receipts ?? []).length > 0} fallback={<p class="dim">Nothing has been erased.</p>}>
          <div class="ops-table-wrap">
            <table class="ops-table">
              <thead><tr><th>When</th><th>What</th><th>Why</th><th>Keys destroyed</th><th>Stored items deleted</th><th>Signature</th><th>Receipt</th></tr></thead>
              <tbody>
                <For each={[...(receipts()?.receipts ?? [])].reverse()}>
                  {(receipt) => (
                    <tr>
                      <td>{day(receipt.at)}</td>
                      <td>{SCOPE[receipt.scope] ?? receipt.scope}</td>
                      <td>{CAUSE[receipt.cause] ?? receipt.cause}</td>
                      <td>{receipt.keys_destroyed}</td>
                      <td>{receipt.objects_deleted}</td>
                      <td>{receipt.verifies ? "Verified" : "Does not verify"}</td>
                      <td class="mono">{receipt.id}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </section>
    </>
  );
}

/** Conversation detail › Lifecycle: where it is in its life, and its hold. */
export function LifecycleTab(props: { sessionId: string }) {
  const [life, { refetch }] = createResource(() => props.sessionId, (id) => api.conversationLifecycle(id));
  const [guests, { refetch: reread }] = createResource(() => props.sessionId, (id) => api.conversationGuests(id));
  const [busy, setBusy] = createSignal(false);
  const eraseGuest = async (principal: string, name: string) => {
    if (!confirmDestructive(`Erase everything ${name} wrote in this conversation and in comments on its files? The conversation stays, with a line where each message was. This cannot be undone.`)) return;
    setBusy(true);
    try {
      await api.eraseGuest(props.sessionId, principal);
      pushToast("info", `What ${name} wrote was erased. The receipt is under Conversations.`);
      void reread();
    } catch (err) {
      pushToast("alert", told(err));
    } finally {
      setBusy(false);
    }
  };
  const hold = async (held: boolean) => {
    setBusy(true);
    try {
      await api.holdConversation(props.sessionId, held);
      pushToast("info", held ? "Conversation put on hold" : "Hold released");
      void refetch();
    } catch (err) {
      pushToast("alert", told(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section class="panel">
      <Show when={!life.error} fallback={<p class="dim">Could not read this conversation's lifecycle: {`${life.error}`}</p>}>
        <Show when={life()} fallback={<p class="dim">Reading…</p>}>
          {(now) => (
            <>
              <h2>{now().trashed_at ? "In the trash" : now().archived ? "Archived" : "In use"}</h2>
              <dl class="lifecycle-facts">
                <dt>Kept for</dt>
                <dd>
                  <Show when={now().trashed_at} fallback={`Until someone moves it to the trash. It is erased ${now().trash_days} days after that.`}>
                    {now().held ? "On hold: it is not erased while the hold stands." : `Until ${day(now().erase_on)}, then erased for good.`}
                  </Show>
                </dd>
                <dt>Rule</dt>
                <dd>The default retention rule for every conversation: {now().trash_days} days in the trash.</dd>
                <dt>Hold</dt>
                <dd>{now().held ? "On hold. Neither a person nor the end of its time in the trash erases it." : "None."}</dd>
              </dl>
              <div class="lifecycle-actions">
                <button class="button small ghost" disabled={busy()} onClick={() => void hold(!now().held)}>{now().held ? "Release hold" : "Put on hold"}</button>
              </div>
              <Show when={(guests()?.guests ?? []).length > 0}>
                <h3>Guests who wrote here</h3>
                <p class="dim">A guest joined through a link you shared. Erasing what one wrote leaves the conversation and everyone else's messages.</p>
                <div class="ops-table-wrap">
                  <table class="ops-table">
                    <tbody>
                      <For each={guests()?.guests ?? []}>
                        {(guest) => (
                          <tr>
                            <td>{guest.name || "A guest"}</td>
                            <td><button class="button small danger ghost" disabled={busy() || now().held} onClick={() => void eraseGuest(guest.principal, guest.name || "this guest")}>Erase what they wrote</button></td>
                          </tr>
                        )}
                      </For>
                    </tbody>
                  </table>
                </div>
              </Show>
            </>
          )}
        </Show>
      </Show>
    </section>
  );
}
