import { createSignal, onCleanup, onMount, Show, For } from "solid-js";
import { backend } from "../store";
import { refreshBackend } from "../App";
import { host } from "../host";
import type { WorkspaceReview } from "../types";
import DirectoryPicker from "./DirectoryPicker";
import Icon from "./Icon";

/**
 * Sign in (where that applies), choose a workspace, and decide about it —
 * three separate decisions, deliberately.
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
 *
 * The web host adds a third step in front: a session has to exist before
 * any of this is reachable. Folder selection there is a *server-side*
 * browser, because the filesystem that matters is the one the agent runs
 * on (docs/design/48-web-client.md §5).
 */
export default function WorkspaceGate() {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [review, setReview] = createSignal<WorkspaceReview | null>(null);
  const [browsing, setBrowsing] = createSignal(false);

  // `null` = not yet known. Only hosts that can authenticate have a
  // meaningful answer; the desktop holds its token already.
  const [authed, setAuthed] = createSignal<boolean | null>(host.authenticate ? null : true);
  const [token, setToken] = createSignal("");

  onMount(() => {
    if (!host.sessionStatus) return;
    void host
      .sessionStatus()
      .then((s) => setAuthed(s.authenticated))
      .catch(() => setAuthed(false));
  });

  // Belt and braces: the host's own info event normally flips this view,
  // and the poll guarantees the transition even if that event is missed.
  onMount(() => {
    let stop = false;
    const tick = async () => {
      // Never probe before we know a session exists: on a host that
      // authenticates, `authed()` starts `null` (unknown), and polling
      // through that window just fires 401s at a server that is behaving
      // correctly.
      if (stop || backend().ready || authed() !== true) return;
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
      const info = await host.info();
      if (!info.ready && info.boot_error) setError(info.boot_error);
    } catch (e) {
      console.error("readBootError: host.info failed", e);
    }
  };

  const signIn = async (event: Event) => {
    event.preventDefault();
    if (!host.authenticate || !token().trim()) return;
    setBusy(true);
    setError(null);
    try {
      await host.authenticate(token().trim());
      setToken(""); // never keep it around after the exchange
      setAuthed(true);
      await refreshBackend();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  /** Decide about `dir`, then open it. */
  const consider = async (dir: string) => {
    setBusy(true);
    setError(null);
    try {
      const reviewed = await host.reviewWorkspace(dir);
      // Nothing privileged to decide about, and nothing already on record
      // to honour: just open it. Prompting here would train people to
      // click through the prompt that does matter.
      if (!reviewed.requests_privilege || reviewed.trusted) {
        await start(dir, null);
        return;
      }
      setReview(reviewed);
      setBrowsing(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      await readBootError();
    } finally {
      setBusy(false);
    }
  };

  const pick = async () => {
    // A host without native dialogs browses the server's filesystem
    // instead — the capability decides the interaction, not the platform.
    if (!host.can("native-dialogs")) {
      setBrowsing(true);
      return;
    }
    const dir = await host.pickWorkspace();
    if (typeof dir === "string") await consider(dir);
  };

  /** `null` uses the decision already on record; a boolean is a fresh answer. */
  const start = async (cwd: string, trust: boolean | null) => {
    setBusy(true);
    setError(null);
    try {
      const info = await host.openWorkspace(cwd, trust ?? undefined);
      const ready = await refreshBackend(info);
      if (!ready) await readBootError();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      await readBootError();
    } finally {
      setBusy(false);
      setReview(null);
      setBrowsing(false);
    }
  };

  return (
    <div class="gate">
      <div class="gate-card" classList={{ wide: browsing() }}>
        <div class="gate-mark"><img src={`${import.meta.env.BASE_URL}vak-icon.png`} alt="" /></div>

        <Show
          when={authed() !== false}
          fallback={
            <form onSubmit={signIn}>
              <h1>Sign in</h1>
              <p class="gate-lead">
                This server is protected by its access token — the one it printed
                on startup, or the <code>VAK_GATEWAY_TOKEN</code> in its
                environment.
              </p>
              <input
                class="gate-token"
                type="password"
                autocomplete="current-password"
                placeholder="Access token"
                aria-label="Access token"
                value={token()}
                onInput={(e) => setToken(e.currentTarget.value)}
              />
              <Show when={error()}><div class="gate-err">{error()}</div></Show>
              <button class="btn primary lg" type="submit" disabled={busy() || !token().trim()}>
                {busy() ? "Signing in…" : "Sign in"}
              </button>
              <p class="gate-note">
                The token is exchanged for a session cookie and never stored in
                this page. Sessions expire on their own; sign out ends one early.
              </p>
            </form>
          }
        >
          <Show when={review()} fallback={
            <Show when={browsing()} fallback={
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
              <>
                <h1>Open a project</h1>
                <p class="gate-lead">
                  Folders on the machine running Vak
                  <Show when={backend().cwd}>{" "}— currently {backend().cwd}</Show>.
                </p>
                <Show when={error()}><div class="gate-err">{error()}</div></Show>
                <DirectoryPicker disabled={busy()} onPick={(dir) => void consider(dir)} />
                <button class="btn subtle" disabled={busy()} onClick={() => setBrowsing(false)}>
                  Cancel
                </button>
              </>
            </Show>
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
        </Show>
      </div>
    </div>
  );
}
