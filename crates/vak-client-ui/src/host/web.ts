// The browser host (docs/design/48-web-client.md §3.1).
//
// Everything is same-origin: the client bundle is served by the very
// server it talks to, under `/app`. There is no base URL to configure and
// no bearer token the page ever holds — auth is an HttpOnly `vak_session`
// cookie set by `POST /auth/login`, which is also the only channel that
// works for `EventSource` (it cannot set headers).

import type { BackendInfo, WorkspaceReview } from "../types";
import type { Host, HostFeature, SaveOutcome, TerminalTransport } from "./port";
import { restartStream, watchHost } from "../streamHub";

/** Same-origin fetch that always carries the session cookie. */
async function call<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    ...init,
    credentials: "same-origin",
    headers: {
      "Content-Type": "application/json",
      ...(init?.headers ?? {}),
    },
  });
  const text = await res.text();
  let parsed: unknown = null;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {
    parsed = text;
  }
  if (!res.ok) {
    throw new Error((parsed as { error?: string })?.error ?? `${res.status} ${res.statusText}`);
  }
  return parsed as T;
}

/** Terminal availability is a server decision (`[server.web] terminal`),
 *  reported by `GET /host`, so it is read once at boot and cached. A
 *  remote host answers `false` unless an operator deliberately enabled it. */
let terminalAllowed = false;

export const activeHost: Host = {
  kind: "web",

  can(feature: HostFeature): boolean {
    switch (feature) {
      // The filesystem the operator cares about is the SERVER's, so a
      // native picker would be listing the wrong machine entirely.
      case "native-dialogs":
        return false;
      case "terminal":
        return terminalAllowed;
      case "multi-workspace":
        return true;
      case "system-notifications":
        return typeof Notification !== "undefined";
      // No menu bar, no login item, no background lifecycle to own.
      case "tray":
        return false;
      case "microphone":
        return typeof navigator !== "undefined" && !!navigator.mediaDevices?.getUserMedia;
      default:
        return false;
    }
  },

  async info(): Promise<BackendInfo> {
    const info = await call<BackendInfo>("/host");
    terminalAllowed = info.terminal === true;
    return info;
  },

  onInfoChanged(handler: (info: BackendInfo) => void): () => void {
    // Host-level changes (a workspace opened in another tab, a failed
    // boot) ride the tab's shared stream rather than being polled.
    return watchHost((data) => {
      const info = data as BackendInfo;
      terminalAllowed = info.terminal === true;
      handler(info);
    });
  },

  openWorkspace(cwd: string, trust?: boolean): Promise<BackendInfo> {
    return call<BackendInfo>("/workspaces/open", {
      method: "POST",
      body: JSON.stringify({ path: cwd, trust: trust ?? null }),
    });
  },

  reviewWorkspace(cwd: string): Promise<WorkspaceReview> {
    return call<WorkspaceReview>("/onboarding/workspace-review", {
      method: "POST",
      body: JSON.stringify({ path: cwd }),
    });
  },

  // No native picker here by design; the UI checks `can("native-dialogs")`
  // and renders the server-side directory browser instead (see
  // (components/DirectoryPicker.tsx, backed by api.listDirectory).
  async pickWorkspace(): Promise<string | null> {
    return null;
  },

  async saveText(suggestedName: string, contents: string, mime = "text/plain"): Promise<SaveOutcome> {
    const url = URL.createObjectURL(new Blob([contents], { type: mime }));
    const a = document.createElement("a");
    a.href = url;
    a.download = suggestedName;
    a.click();
    URL.revokeObjectURL(url);
    return { kind: "downloaded" };
  },

  async notify(title: string, body: string, route?: string): Promise<void> {
    if (typeof Notification === "undefined") return;
    try {
      let permission = Notification.permission;
      // Asked at the moment a notification is first actually warranted —
      // never on load, which is the request everyone denies reflexively.
      if (permission === "default") permission = await Notification.requestPermission();
      if (permission !== "granted") return;
      const notification = new Notification(title, { body, icon: "/app/vak-icon.png" });
      // Clicking it must land on the thing it is about. Without this the
      // tab merely surfaces, on whatever session was last open — which for
      // an approval means hunting for the card the alert was about.
      notification.onclick = () => {
        window.focus();
        if (route) window.location.hash = route;
        notification.close();
      };
    } catch {
      /* optional */
    }
  },

  openAdmin(route: string): void {
    // Same origin: the operations console is served by this same process.
    window.open(`/admin${route}`, "_blank", "noopener");
  },

  async terminal(cwd: string): Promise<TerminalTransport | null> {
    if (!terminalAllowed) return null;
    const scheme = window.location.protocol === "https:" ? "wss" : "ws";
    const socket = new WebSocket(
      `${scheme}://${window.location.host}/pty?cwd=${encodeURIComponent(cwd)}`,
    );
    socket.binaryType = "arraybuffer";

    let dataHandler: ((bytes: Uint8Array) => void) | null = null;
    let exitHandler: (() => void) | null = null;
    socket.onmessage = (event) => {
      if (event.data instanceof ArrayBuffer) dataHandler?.(new Uint8Array(event.data));
    };
    socket.onclose = () => exitHandler?.();

    // Input and resize share one socket, so they need to be told apart.
    // A resize is a JSON text frame; keystrokes are binary. Nothing the
    // user types can be mistaken for a control message.
    const send = (payload: ArrayBufferView | string) => {
      if (socket.readyState === WebSocket.OPEN) socket.send(payload);
    };
    const queue: (ArrayBufferView | string)[] = [];
    socket.onopen = () => {
      for (const item of queue.splice(0)) send(item);
    };
    const emit = (payload: ArrayBufferView | string) => {
      if (socket.readyState === WebSocket.OPEN) send(payload);
      else queue.push(payload);
    };

    return {
      onData(handler) {
        dataHandler = handler;
      },
      onExit(handler) {
        exitHandler = handler;
      },
      write(data) {
        emit(new TextEncoder().encode(data));
      },
      resize(cols, rows) {
        emit(JSON.stringify({ resize: { cols, rows } }));
      },
      close() {
        socket.close();
      },
    };
  },

  /** Exchange the bearer token for the session cookie. The token is never
   *  stored client-side: it goes straight into this request, and what comes
   *  back is HttpOnly — script can neither read it nor exfiltrate it. */
  async authenticate(token: string): Promise<void> {
    await call("/auth/login", { method: "POST", body: JSON.stringify({ token }) });
    // A stream refused before sign-in is waiting out its retry backoff.
    restartStream(false);
  },

  sessionStatus(): Promise<{ authenticated: boolean; expires_at?: string | null }> {
    return call("/auth/session");
  },

  async logout(): Promise<void> {
    await call("/auth/logout", { method: "POST", body: "{}" });
  },

  async getAutostart(): Promise<boolean> {
    return false;
  },

  async setAutostart(_enabled: boolean): Promise<void> {},
};
