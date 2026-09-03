// The Tauri desktop host (docs/design/48-web-client.md §3.1).
//
// Behaviourally identical to what the client did before the port existed:
// every call here is the same `invoke`/plugin call the components used to
// make inline. Nothing about the desktop's behaviour changes by routing
// through the interface — that is the point of extracting it.

import { invoke } from "@tauri-apps/api/core";
import { Channel } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";

import type { BackendInfo, WorkspaceReview } from "../types";
import type { Host, HostFeature, SaveOutcome, TerminalTransport } from "./port";

export const activeHost: Host = {
  kind: "desktop",

  can(feature: HostFeature): boolean {
    // Every capability, which is exactly what makes the desktop the
    // reference host: a feature missing here would be a bug, not a
    // deployment choice.
    const supported: HostFeature[] = [
      "native-dialogs",
      "terminal",
      "multi-workspace",
      "system-notifications",
      "tray",
    ];
    return supported.includes(feature);
  },

  info(): Promise<BackendInfo> {
    return invoke<BackendInfo>("backend_info");
  },

  onInfoChanged(handler: (info: BackendInfo) => void): () => void {
    // `backend-ready` is emitted by the shell when it finishes booting a
    // workspace's embedded server.
    const pending = listen<BackendInfo>("backend-ready", (event) => handler(event.payload));
    let stopped = false;
    void pending.then((un) => {
      if (stopped) un();
    });
    return () => {
      stopped = true;
      void pending.then((un) => un());
    };
  },

  openWorkspace(cwd: string, trust?: boolean): Promise<BackendInfo> {
    return invoke<BackendInfo>("start_backend", { cwd, trust: trust ?? null });
  },

  reviewWorkspace(cwd: string): Promise<WorkspaceReview> {
    return invoke<WorkspaceReview>("review_workspace", { cwd });
  },

  async pickWorkspace(): Promise<string | null> {
    const dir = await openDialog({ directory: true, multiple: false, title: "Open a project" });
    return typeof dir === "string" ? dir : null;
  },

  async saveText(suggestedName: string, contents: string): Promise<SaveOutcome> {
    const extension = suggestedName.split(".").pop() ?? "txt";
    const path = await saveDialog({
      title: "Save",
      defaultPath: suggestedName,
      filters: [{ name: extension.toUpperCase(), extensions: [extension] }],
    });
    if (!path) return { kind: "cancelled" };
    await invoke("export_text_file", { path, contents });
    return { kind: "saved", path };
  },

  async notify(title: string, body: string, _route?: string): Promise<void> {
    try {
      const n = await import("@tauri-apps/plugin-notification");
      let granted = await n.isPermissionGranted();
      if (!granted) granted = (await n.requestPermission()) === "granted";
      if (granted) n.sendNotification({ title, body });
    } catch {
      /* notifications are optional, never a reason to break a run */
    }
  },

  openAdmin(route: string): void {
    // Route is chosen by our own UI, never by remote content; the shell
    // appends it to a loopback URL it builds itself.
    void invoke("open_admin", { route });
  },

  async terminal(cwd: string): Promise<TerminalTransport | null> {
    let dataHandler: ((bytes: Uint8Array) => void) | null = null;
    let exitHandler: (() => void) | null = null;

    const channel = new Channel<number[]>();
    channel.onmessage = (bytes) => dataHandler?.(new Uint8Array(bytes));

    // Size is corrected by the first `resize` once xterm has measured
    // itself; these are only what the shell starts life believing.
    const ptyId = await invoke<string>("spawn_pty", { cwd, cols: 80, rows: 24, onData: channel });

    const unlisten = listen<string>("pty-exit", (e) => {
      if (e.payload === ptyId) exitHandler?.();
    });

    let closed = false;
    return {
      onData(handler) {
        dataHandler = handler;
      },
      onExit(handler) {
        exitHandler = handler;
      },
      write(data) {
        void invoke("pty_write", { id: ptyId, data: Array.from(new TextEncoder().encode(data)) });
      },
      resize(cols, rows) {
        void invoke("pty_resize", { id: ptyId, cols, rows });
      },
      close() {
        if (closed) return;
        closed = true;
        void unlisten.then((un) => un());
        void invoke("pty_close", { id: ptyId }).catch(() => {});
      },
    };
  },
};
