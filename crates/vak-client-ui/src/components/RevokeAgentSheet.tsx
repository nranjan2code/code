import { Show, createResource, createSignal } from "solid-js";
import * as api from "../api";
import Sheet from "./Sheet";

const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/**
 * Revoking an Agent (doc 74 §2.3, §6.2 A14): for when it may have been
 * compromised. Shows what it holds that will be cut and what is kept, and
 * takes its name typed. The server checks the name again.
 */
export default function RevokeAgentSheet(props: {
  agent: api.Agent;
  onClose: () => void;
  onRevoked: (left: string[]) => void;
}) {
  const [reach] = createResource(() => props.agent.id, api.agentLifecycle);
  const [typed, setTyped] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [problem, setProblem] = createSignal("");
  const ready = () => typed().trim() === props.agent.name.trim();
  const revoke = async () => {
    if (!ready() || busy()) return;
    setBusy(true);
    setProblem("");
    try {
      const done = await api.revokeAgent(props.agent.id, typed().trim());
      props.onRevoked(done.not_removed);
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
      title={`Revoke ${props.agent.name}?`}
      subtitle="Use this if the agent may have been compromised. It stops at once and cannot be resumed."
      onClose={props.onClose}
      busy={busy()}
      footer={<>
        <button type="button" class="btn" disabled={busy()} onClick={props.onClose}>Cancel</button>
        <button type="button" class="btn danger" disabled={!ready() || busy()} onClick={() => void revoke()}>{busy() ? "Revoking…" : "Revoke agent"}</button>
      </>}
    >
      <div class="erase-sheet">
        <section class="confirm-modal-review" aria-label="What revoking does">
          <strong>This removes, now</strong>
          <Show when={reach()} fallback={<ul><li>{reach.error ? "Its sign-in details, bot tokens and connected accounts" : "Checking what it holds…"}</li></ul>}>
            {(held) => (
              <ul>
                <Show when={held().bots > 0}><li>{count(held().bots, "bot token", "bot tokens")}, so its bots stop answering</li></Show>
                <Show when={held().accounts > 0}><li>{count(held().accounts, "connected mail or calendar account", "connected mail or calendar accounts")}</li></Show>
                <Show when={held().secrets > 0}><li>{count(held().secrets, "saved key or password of its own", "saved keys or passwords of its own")}</li></Show>
                <Show when={held().automations > 0}><li>Its {count(held().automations, "automation", "automations")}, which will no longer run</li></Show>
                <li>Anything it is doing right now, and every later request to it</li>
              </ul>
            )}
          </Show>
          <strong>This keeps</strong>
          <ul>
            <li>Its conversations, files and memory, which you can still read</li>
            <li>Copies of mail and events already in conversations, until you delete them</li>
          </ul>
        </section>
        <label class="erase-confirm">
          <span>Type <strong>{props.agent.name}</strong> to confirm</span>
          <input class="settings-input" autocomplete="off" spellcheck={false} value={typed()} onInput={(event) => setTyped(event.currentTarget.value)} onKeyDown={(event) => { if (event.key === "Enter") void revoke(); }} />
        </label>
        <Show when={problem()}><p class="erase-note error" role="alert">{problem()}</p></Show>
      </div>
    </Sheet>
  );
}
