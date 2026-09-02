import { createSignal, onCleanup, onMount, Show, For } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { backend } from "../store";
import { refreshBackend } from "../App";
import type { BackendInfo } from "../types";
import Icon from "./Icon";

interface WorkspaceReview {
  path: string;
  git: boolean;
  requests_privilege: boolean;
  privileges: string[];
  trusted: boolean;
}

/**
 * Choose a workspace, and decide about it separately.
 *
 * Replaces `ProjectGate`, where picking a folder *was* the act of trusting
 * it: the backend booted with `Core::new_with_trust(cwd, true)` and the
 * project's `.env` joined the process environment, so opening a repository
 * silently granted whatever its `.vak/config.toml` asked for — hooks, MCP
 * servers, a redirected provider endpoint (docs/design/46, Step 2, and
 * security invariant 2).
 *
 * Now selection and consent are two decisions. A folder that asks for
 * nothing privileged opens with no prompt at all; one that does gets a
 * review naming exactly what it wants, read as text without loading it.
 */
export default function WorkspaceGate() {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [review, setReview] = createSignal<WorkspaceReview | null>(null);

  // Belt and braces: the backend-ready event normally flips this view, and
  // the poll guarantees the transition even if that event is missed.
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
      if (typeof dir !== "string") return;
      const reviewed = await invoke<WorkspaceReview>("review_workspace", { cwd: dir });
      // Nothing privileged to decide about, and nothing already on record
      // to honour: just open it. Prompting here would train people to
      // click through the prompt that does matter.
      if (!reviewed.requests_privilege || reviewed.trusted) {
        await start(dir, null);
        return;
      }
      setReview(reviewed);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      await readBootError();
    } finally {
      setBusy(false);
    }
  };

  /** `null` uses the decision already on record; a boolean is a fresh answer. */
  const start = async (cwd: string, trust: boolean | null) => {
    setBusy(true);
    setError(null);
    try {
      await invoke("start_backend", { cwd, trust });
      const ready = await refreshBackend();
      if (!ready) await readBootError();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      await readBootError();
    } finally {
      setBusy(false);
      setReview(null);
    }
  };

  return (
    <div class="gate">
      <div class="gate-card">
        <div class="gate-mark"><img src="/vak-icon.png" alt="" /></div>

        <Show when={review()} fallback={
          <>
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
            <button class="btn primary lg" disabled={busy()} onClick={() => void pick()}>
              <Icon name="folder" /> {busy() ? "Opening workspace…" : "Open a project"}
            </button>
            <p class="gate-note">
              Shared defaults stay global and are inherited here; project settings
              override only that folder. Configuration and secrets stay on this device.
            </p>
          </>
        }>
          {(r) => (
            <>
              <h1>Trust this folder?</h1>
              <p class="gate-lead mono">{r().path}</p>
              <p>Its own settings ask for:</p>
              <ul class="gate-privileges">
                <For each={r().privileges}>{(p) => <li>{p}</li>}</For>
              </ul>
              <p class="gate-note">
                Opening safely runs everything normally and ignores those settings.
                Trusting lets them take effect, including commands they can run.
                You can change this later.
              </p>
              <Show when={error()}><div class="gate-err">{error()}</div></Show>
              <div class="gate-actions">
                <button class="btn primary" disabled={busy()} onClick={() => void start(r().path, false)}>
                  Open safely
                </button>
                <button class="btn" disabled={busy()} onClick={() => void start(r().path, true)}>
                  Trust this folder
                </button>
                <button class="btn subtle" disabled={busy()} onClick={() => setReview(null)}>
                  Cancel
                </button>
              </div>
            </>
          )}
        </Show>
      </div>
    </div>
  );
}
