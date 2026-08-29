import { createEffect, onCleanup, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Channel } from "@tauri-apps/api/core";
import { backend } from "../store";
import type { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

interface Live {
  ptyId: string;
  term: Terminal;
  dispose: () => void;
}

const live = new Map<string, Live>();
const pending = new Set<string>();

export default function TerminalPane(props: { sessionId: string | null }) {
  let host!: HTMLDivElement;

  let prevSid: string | null = null;

  createEffect(() => {
    const sid = props.sessionId;
    if (!host) return;
    // When the session changes, dispose the old terminal so PTY handles and
    // xterm DOM roots don't accumulate across switches.
    if (prevSid && prevSid !== sid) {
      const old = live.get(prevSid);
      if (old) old.dispose();
    }
    prevSid = sid;
    if (!sid) return;
    if (live.has(sid) || pending.has(sid)) return;
    void mount(sid, host);
  });

  async function mount(sid: string, el: HTMLDivElement) {
    pending.add(sid);
    try {
      const [{ Terminal }, { FitAddon }] = await Promise.all([
        import("@xterm/xterm"),
        import("@xterm/addon-fit"),
      ]);

      const term = new Terminal({
        fontFamily: '"SF Mono", ui-monospace, Menlo, Consolas, monospace',
        fontSize: 12.5,
        theme: {
          background: "#12121a",
          foreground: "#c0caf5",
          cursor: "#7aa2f7",
          selectionBackground: "#33467c",
        },
        cursorBlink: true,
      });
      const fit = new FitAddon();
      term.loadAddon(fit);
      // Detach any previously mounted xterm DOM so sessions get their own.
      for (const child of Array.from(el.children)) child.remove();
      term.open(el);
      fit.fit();

      const onData = new Channel<number[]>();
      onData.onmessage = (bytes) => term.write(new Uint8Array(bytes));

      const ptyId = await invoke<string>("spawn_pty", {
        cwd: backend().cwd ?? ".",
        cols: term.cols,
        rows: term.rows,
        onData,
      });

      const inputSub = term.onData((d) => {
        void invoke("pty_write", {
          id: ptyId,
          data: Array.from(new TextEncoder().encode(d)),
        });
      });
      const ro = new ResizeObserver(() => {
        try {
          fit.fit();
          void invoke("pty_resize", { id: ptyId, cols: term.cols, rows: term.rows });
        } catch {
          /* pane hidden */
        }
      });
      ro.observe(el);

      const unExitPromise = listen<string>("pty-exit", (e) => {
        if (e.payload === ptyId) term.writeln("\r\n[shell exited]");
      });

      const dispose = () => {
        inputSub.dispose();
        ro.disconnect();
        void unExitPromise.then((f) => f());
        term.dispose();
        live.delete(sid);
      };
      live.set(sid, { ptyId, term, dispose });
      onCleanup(dispose);
    } catch (e) {
      el.textContent = `failed to start shell: ${e}`;
    } finally {
      pending.delete(sid);
    }
  }

  onCleanup(() => {
    live.forEach((l) => l.dispose());
    live.clear();
  });

  return (
    <div class="termpane">
      <div class="dock-head">
        <span>Terminal</span>
        <Show when={!props.sessionId}>
          <span class="hint">select a session</span>
        </Show>
      </div>
      <div class="term-host" ref={host} />
    </div>
  );
}
