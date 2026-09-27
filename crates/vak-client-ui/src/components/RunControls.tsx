import { createSignal, onCleanup, onMount, Show } from "solid-js";
import * as api from "../api";
import { setNotice } from "../store";
import Icon from "./Icon";
import Sheet from "./Sheet";

export default function RunControls(props: { sessionId: string }) {
  const [paused, setPaused] = createSignal(false);
  const [revision, setRevision] = createSignal(0);
  const [busy, setBusy] = createSignal(false);
  const [controlError, setControlError] = createSignal("");
  const [changing, setChanging] = createSignal(false);
  const [changeText, setChangeText] = createSignal("");
  const [changeKind, setChangeKind] = createSignal("replan");
  const [changeResult, setChangeResult] = createSignal("");
  const refresh = () => void api.controlState(props.sessionId).then((state) => {
    setPaused(state.paused);
    setRevision(state.revision);
    setControlError("");
  }).catch((error) => {
    setControlError(`Control state unavailable: ${error instanceof Error ? error.message : String(error)}`);
  });
  onMount(() => {
    refresh();
    const timer = window.setInterval(refresh, 2000);
    onCleanup(() => window.clearInterval(timer));
  });
  const toggle = async () => {
    if (busy()) return;
    setBusy(true);
    try {
      if (paused()) await api.resumeRun(props.sessionId);
      else await api.pauseRun(props.sessionId);
      // Read back the control state so the label reflects the server's
      // revision rather than a local optimistic guess.
      await api.controlState(props.sessionId).then((state) => {
        setPaused(state.paused);
        setRevision(state.revision);
      });
    } catch (error) {
      setNotice({ kind: "error", text: `Could not ${paused() ? "resume" : "pause"} this task: ${error instanceof Error ? error.message : String(error)}` });
    } finally {
      setBusy(false);
    }
  };
  const submitChange = async () => {
    const text = changeText().trim();
    if (!text || busy()) return;
    setBusy(true);
    try {
      const result = await api.planChange(props.sessionId, `${changeKind()}: ${text}`, "human", revision());
      if (result.decision === "requires_human") {
        setChangeResult(`Human review required · plan v${result.revision}`);
      } else {
        setRevision(result.revision);
        setChangeResult(`${result.decision} · plan v${result.revision}`);
      }
      setChangeText("");
    } catch (error) {
      const message = error instanceof Error ? error.message : "Plan change failed";
      if (message.includes("409")) {
        refresh();
        setChangeResult("Plan changed elsewhere; review the new revision");
      } else {
        setChangeResult(message);
      }
    } finally {
      setBusy(false);
    }
  };
  return <>
    <button type="button" class="composer-pause" disabled={busy()} onClick={() => void toggle()} aria-label={paused() ? "Resume task" : "Pause task"} title={controlError() || (paused() ? "Resume paused task" : "Pause at the next safe boundary")}>
      {paused() ? "Resume" : "Pause"}
    </button>
    <Show when={revision() > 0}>
      <button type="button" class="composer-plan" title="Change plan" aria-label="Change plan" onClick={() => setChanging(true)}><Icon name="tune" size={15} /></button>
    </Show>
    <Show when={changing() && revision() > 0}>
      <Sheet title="Change plan" onClose={() => setChanging(false)} size="narrow">
      <form class="plan-change" onSubmit={(event) => { event.preventDefault(); void submitChange(); }}>
        <select aria-label="Plan change type" value={changeKind()} onChange={(event) => setChangeKind(event.currentTarget.value)}>
          <option value="replan">Replan</option>
          <option value="add requirement">Add requirement</option>
          <option value="remove requirement">Remove requirement</option>
          <option value="reprioritize">Reprioritize</option>
        </select>
        <input aria-label="Plan change" value={changeText()} placeholder="Add, remove, or reprioritize work…" onInput={(event) => setChangeText(event.currentTarget.value)} />
        <button class="run-control" type="submit" disabled={busy() || !changeText().trim()}>Submit</button>
        <Show when={changeResult()}><span role="status">{changeResult()}</span></Show>
      </form>
      </Sheet>
    </Show>
  </>;
}
