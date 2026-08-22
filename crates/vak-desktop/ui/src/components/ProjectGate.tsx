import { createSignal, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

export default function ProjectGate() {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const pick = async () => {
    setBusy(true);
    setError(null);
    try {
      const dir = await open({ directory: true, multiple: false, title: "Open a project" });
      if (typeof dir === "string") {
        await invoke("start_backend", { cwd: dir });
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="gate">
      <div class="gate-card">
        <div class="gate-mark">◆</div>
        <h1>vakcoder</h1>
        <p>Pick a project folder to start the agent.</p>
        <Show when={error()}>
          <div class="gate-err">{error()}</div>
        </Show>
        <button class="btn primary lg" onClick={() => void pick()} disabled={busy()}>
          {busy() ? "Starting…" : "Choose folder…"}
        </button>
        <p class="gate-note">The agent runs locally. Secrets stay in .env files.</p>
      </div>
    </div>
  );
}
