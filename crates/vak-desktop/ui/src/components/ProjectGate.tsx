import { createSignal, onCleanup, onMount, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { backend, providers } from "../store";
import { refreshBackend } from "../App";
import type { BackendInfo } from "../types";
import Icon from "./Icon";

/**
 * Project gate: shown only while no backend is attached to a workspace yet.
 * Provider/key setup is NOT gated here any more — once a workspace is open
 * the app renders and a dismissible SetupCard handles credentials
 * (docs/design/29-personal-os.md).
 */
export default function ProjectGate() {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  // Belt and braces: while the gate is up, poll the shell for backend state.
  // The backend-ready event normally flips this view instantly; the poll
  // guarantees the transition even if that event is missed.
  //
  // This deliberately lives in onMount, not createEffect: the poll writes
  // backend(), so a tracking scope that also reads it would tear down and
  // rebuild the timer on every tick instead of polling steadily.
  onMount(() => {
    let stop = false;
    const tick = async () => {
      if (stop || backend().ready) return;
      await refreshBackend();
    };
    const t = setInterval(() => void tick(), 800);
    void tick();
    onCleanup(() => {
      stop = true;
      clearInterval(t);
    });
  });

  const readBootError = async () => {
    try {
      const info = await invoke<BackendInfo>("backend_info");
      if (!info.ready && info.boot_error) setError(info.boot_error);
    } catch (e) {
      console.error("readBootError: backend_info failed", e);
    }
  };

  const pick = async () => {
    setBusy(true);
    setError(null);
    try {
      const dir = await open({ directory: true, multiple: false, title: "Open a project" });
      if (typeof dir === "string") {
        await invoke("start_backend", { cwd: dir });
        // Flip proactively; do not trust the event alone.
        const ready = await refreshBackend();
        if (!ready) await readBootError();
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      await readBootError();
    } finally {
      setBusy(false);
    }
  };

  // The picker is the whole gate now; a loaded provider snapshot just means
  // the workspace is about to take over.
  // A provider snapshot may arrive before a workspace backend (for example
  // after a desktop-service handover). It must never suppress the only
  // recovery control while the backend is unavailable.
  const showPicker = () => !backend().ready || !providers();

  return (
    <div class="gate">
      <div class="gate-card">
        <div class="gate-mark"><img src="/vak-icon.png" alt="" /></div>
        <Show when={showPicker()}>
          <h1>Vak</h1>
          <p class="gate-lead">Your code, your machine, your agent.</p>
          <div class="gate-features">
            <span><Icon name="check" size={15} /> Isolated tasks and worktrees</span>
            <span><Icon name="check" size={15} /> Review every change before keeping it</span>
            <span><Icon name="check" size={15} /> Local-first and fully inspectable</span>
          </div>
          <Show when={error() || backend().boot_error}>
            <div class="gate-err">{error() ?? backend().boot_error}</div>
          </Show>
          <button class="btn primary lg" onClick={() => void pick()} disabled={busy()}>
            <Icon name="folder" /> {busy() ? "Opening workspace…" : "Open a project"}
          </button>
          <p class="gate-note">
            Vak prepares this project’s <code>.vak</code> settings layer. Shared
            defaults stay global and are inherited here; project settings only
            override this folder. Configuration and secrets remain on this device.
          </p>
        </Show>
      </div>
    </div>
  );
}
