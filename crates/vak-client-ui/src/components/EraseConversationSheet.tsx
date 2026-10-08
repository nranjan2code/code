import { For, Show, createResource, createSignal } from "solid-js";
import * as api from "../api";
import Sheet from "./Sheet";

/** What erasing reaches, in the words a person uses for it. */
function reach(preview: api.ErasurePreview): string[] {
  const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  const lines = [count(preview.conversations.length, "conversation, with the work it delegated", "conversations, with the work they delegated")];
  if (preview.artifacts.length) lines.push(count(preview.artifacts.length, "draft nobody kept", "drafts nobody kept"));
  if (preview.memory_notes) lines.push(count(preview.memory_notes, "memory note written from it", "memory notes written from it"));
  return lines;
}

/**
 * Deleting a conversation for good (doc 74 §6.3, C2): what it reaches,
 * what it cannot, and the conversation's title typed before anything
 * happens. The server checks the title and the preview again.
 */
export default function EraseConversationSheet(props: {
  sessionId: string;
  title: string;
  onClose: () => void;
  onErased: () => void;
}) {
  const [preview] = createResource(() => props.sessionId, api.erasurePreview);
  const [typed, setTyped] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [problem, setProblem] = createSignal("");
  const ready = () => {
    const shown = preview();
    return Boolean(shown) && !shown!.preview.held && typed().trim() === shown!.confirm;
  };
  const erase = async () => {
    const shown = preview();
    if (!shown || !ready() || busy()) return;
    setBusy(true);
    setProblem("");
    try {
      await api.eraseConversation(props.sessionId, shown.preview.digest, typed().trim());
      props.onErased();
      props.onClose();
    } catch (error) {
      setProblem(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Sheet
      size="narrow"
      title={`Delete “${props.title}” for good?`}
      subtitle="This cannot be undone. Nothing can restore it afterwards."
      onClose={props.onClose}
      busy={busy()}
      footer={<>
        <button type="button" class="btn" disabled={busy()} onClick={props.onClose}>Cancel</button>
        <button type="button" class="btn danger" disabled={!ready() || busy()} onClick={() => void erase()}>{busy() ? "Deleting…" : "Delete for good"}</button>
      </>}
    >
      <Show when={preview()} fallback={<p class="erase-note">{preview.error ? "Could not check what this would delete. Try again." : "Checking what this would delete…"}</p>}>
        {(shown) => (
          <div class="erase-sheet">
            <section class="confirm-modal-review" aria-label="What will be deleted">
              <strong>This deletes</strong>
              <ul><For each={reach(shown().preview)}>{(line) => <li>{line}</li>}</For></ul>
              <strong>This cannot reach</strong>
              <ul>
                <Show when={shown().preview.sent_outside}><li>{shown().preview.sent_outside} message{shown().preview.sent_outside === 1 ? "" : "s"} or change{shown().preview.sent_outside === 1 ? "" : "s"} already sent outside Vakyartha</li></Show>
                <li>Copies the AI services received when they answered</li>
                <li>Backups made before now, until they expire</li>
              </ul>
            </section>
            <Show when={shown().preview.held} fallback={
              <label class="erase-confirm">
                <span>Type <strong>{shown().confirm}</strong> to confirm</span>
                <input class="settings-input" autocomplete="off" spellcheck={false} value={typed()} onInput={(event) => setTyped(event.currentTarget.value)} onKeyDown={(event) => { if (event.key === "Enter") void erase(); }} />
              </label>
            }>
              <p class="erase-note">This conversation is on hold, so it cannot be deleted until the hold is released.</p>
            </Show>
            <Show when={problem()}><p class="erase-note error" role="alert">{problem()}</p></Show>
          </div>
        )}
      </Show>
    </Sheet>
  );
}
