import { createEffect, onCleanup, Show } from "solid-js";
import { backend } from "../store";
import { host, type TerminalTransport } from "../host";
import type { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

interface Live {
  term: Terminal;
  dispose: () => void;
}

const live = new Map<string, Live>();
const pending = new Set<string>();

/**
 * A real shell, on whichever machine the backend is running on.
 *
 * The transport is the host's (docs/design/48-web-client.md §6): a Tauri
 * IPC channel over `portable-pty` on the desktop, a WebSocket on the web.
 * Either way this component only moves bytes and never knows which.
 *
 * On a remote host the terminal is off unless an operator deliberately
 * enabled it — a shell over HTTP is remote code execution, and unlike
 * every other effect in this product it is not mediated by the permission
 * engine. When it is off there is no tab to click at all; a disabled
 * control that cannot explain itself is worse than an absent one.
 */
export default function TerminalPane(props: { sessionId: string | null }) {
  let hostEl!: HTMLDivElement;
  let prevSid: string | null = null;

  createEffect(() => {
    const sid = props.sessionId;
    if (!hostEl) return;
    // When the session changes, dispose the old terminal so PTY handles and
    // xterm DOM roots don't accumulate across switches.
    if (prevSid && prevSid !== sid) live.get(prevSid)?.dispose();
    prevSid = sid;
    if (!sid) return;
    if (live.has(sid) || pending.has(sid)) return;
    void mount(sid, hostEl);
  });

  async function mount(sid: string, el: HTMLDivElement) {
    pending.add(sid);
    let transport: TerminalTransport | null = null;
    try {
      const [{ Terminal }, { FitAddon }] = await Promise.all([
        import("@xterm/xterm"),
        import("@xterm/addon-fit"),
      ]);

      transport = await host.terminal(backend().cwd ?? ".");
      if (!transport) {
        el.innerHTML = `<div class="terminal-disabled" role="status"><strong>Terminal is disabled by this server</strong><span>Ask an operator to enable the loopback terminal in server settings, then reload this task.</span></div>`;
        return;
      }

      const term = new Terminal({
        fontFamily: '"SFMono-Regular", "SF Mono", ui-monospace, Menlo, Consolas, monospace',
        fontSize: 12.5,
        theme: terminalTheme(),
        cursorBlink: true,
      });
      const fit = new FitAddon();
      term.loadAddon(fit);
      // Detach any previously mounted xterm DOM so sessions get their own.
      for (const child of Array.from(el.children)) child.remove();
      term.open(el);
      fit.fit();

      transport.onData((bytes) => term.write(bytes));
      transport.onExit(() => term.writeln("\r\n[shell exited]"));
      transport.resize(term.cols, term.rows);

      const inputSub = term.onData((data) => transport?.write(data));
      const ro = new ResizeObserver(() => {
        try {
          fit.fit();
          transport?.resize(term.cols, term.rows);
        } catch {
          /* pane hidden */
        }
      });
      ro.observe(el);

      const dispose = () => {
        inputSub.dispose();
        ro.disconnect();
        term.dispose();
        live.delete(sid);
        // Ends the shell and releases its handle on the other side.
        transport?.close();
      };
      live.set(sid, { term, dispose });
      onCleanup(dispose);
    } catch (e) {
      transport?.close();
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
      <div class="term-host" ref={hostEl} />
    </div>
  );
}

/**
 * xterm cannot read CSS custom properties, so the active theme's tokens are
 * resolved here and handed over as literals.
 *
 * This used to be a hardcoded Tokyo Night palette — cool blues and purples
 * in a product whose whole visual system is a warm near-monochrome with one
 * terracotta accent (DESIGN.md). It was the most obviously foreign surface
 * in the app, and it stayed foreign in every theme.
 */
function terminalTheme() {
  const read = (token: string, fallback: string) =>
    getComputedStyle(document.documentElement).getPropertyValue(token).trim() || fallback;
  return {
    background: read("--bg", "#171714"),
    foreground: read("--text-soft", "#c4c0b8"),
    cursor: read("--accent", "#df795f"),
    cursorAccent: read("--bg", "#171714"),
    selectionBackground: read("--surface-active", "#30302b"),
    black: read("--surface-raised", "#22221f"),
    red: read("--red", "#d86f72"),
    green: read("--green", "#73a982"),
    yellow: read("--yellow", "#d4a85d"),
    blue: read("--blue", "#7c9fc9"),
    magenta: read("--accent-bright", "#ee9278"),
    cyan: read("--blue", "#7c9fc9"),
    white: read("--text", "#eeeae2"),
    brightBlack: read("--faint", "#8b8880"),
  };
}
