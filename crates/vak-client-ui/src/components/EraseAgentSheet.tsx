import { Show, createResource, createSignal } from "solid-js";
import * as api from "../api";
import Sheet from "./Sheet";

const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/**
 * Deleting everything an Agent holds (plan M7b-c). Offered once the Agent
 * is archived or revoked. Shows what goes and what stays, and takes the
 * Agent's name typed; the server checks the name and the preview again.
 */
export default function EraseAgentSheet(props: {
  agent: api.Agent;
  onClose: () => void;
  onErased: () => void;
}) {
  const [looked] = createResource(() => props.agent.id, api.agentErasurePreview);
  const [typed, setTyped] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [problem, setProblem] = createSignal("");
  const ready = () => {
    const shown = looked();
    return Boolean(shown) && !shown!.preview.held && typed().trim() === shown!.confirm.trim();
  };
  const erase = async () => {
    const shown = looked();
    if (!shown || !ready() || busy()) return;
    setBusy(true);
    setProblem("");
    try {
      await api.eraseAgent(props.agent.id, shown.preview.digest, typed().trim());
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
      title={`Delete everything ${props.agent.name} holds?`}
      subtitle="This cannot be undone. Nothing can restore it afterwards."
      onClose={props.onClose}
      busy={busy()}
      footer={<>
        <button type="button" class="btn" disabled={busy()} onClick={props.onClose}>Cancel</button>
        <button type="button" class="btn danger" disabled={!ready() || busy()} onClick={() => void erase()}>{busy() ? "Deleting…" : "Delete for good"}</button>
      </>}
    >
      <Show when={looked()} fallback={<p class="erase-note">{looked.error ? `This cannot be deleted yet: ${looked.error instanceof Error ? looked.error.message : looked.error}` : "Checking what this would delete…"}</p>}>
        {(shown) => (
          <div class="erase-sheet">
            <section class="confirm-modal-review" aria-label="What will be deleted">
              <strong>This deletes</strong>
              <ul>
                <li>{count(shown().preview.conversations, "conversation", "conversations")} with this agent</li>
                <Show when={shown().preview.documents > 0}><li>{count(shown().preview.documents, "thing it remembers", "things it remembers")}</li></Show>
                <Show when={shown().preview.artifacts > 0}><li>{count(shown().preview.artifacts, "draft nobody kept", "drafts nobody kept")}</li></Show>
                <Show when={shown().preview.automations > 0}><li>{count(shown().preview.automations, "automation", "automations")}</li></Show>
                <Show when={shown().preview.workspace_files > 0}><li>{count(shown().preview.workspace_files, "file", "files")} in the working folder Vakyartha keeps for it</li></Show>
              </ul>
              <strong>This keeps</strong>
              <ul>
                <li>Files of its work that you accepted, saved, starred or shared</li>
                <li>Anything it wrote into a folder of yours</li>
                <li>Messages it already sent, and copies the AI services hold</li>
              </ul>
            </section>
            <Show when={!shown().preview.held} fallback={<p class="erase-note">Something of this agent's is on hold, so it cannot be deleted until the hold is released.</p>}>
              <label class="erase-confirm">
                <span>Type <strong>{shown().confirm}</strong> to confirm</span>
                <input class="settings-input" autocomplete="off" spellcheck={false} value={typed()} onInput={(event) => setTyped(event.currentTarget.value)} onKeyDown={(event) => { if (event.key === "Enter") void erase(); }} />
              </label>
            </Show>
            <Show when={problem()}><p class="erase-note error" role="alert">{problem()}</p></Show>
          </div>
        )}
      </Show>
    </Sheet>
  );
}
