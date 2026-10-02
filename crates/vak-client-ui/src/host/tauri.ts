// The Tauri desktop host (docs/design/48-web-client.md §3.1).
//
// Behaviourally identical to what the client did before the port existed:
// every call here is the same `invoke`/plugin call the components used to
// make inline. Nothing about the desktop's behaviour changes by routing
// through the interface — that is the point of extracting it.

import { invoke } from "@tauri-apps/api/core";
import { Channel } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
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
      "microphone",
      "open-with",
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

  onFullscreenChange(handler: (fullscreen: boolean) => void): () => void {
    // macOS hides the overlaid window controls in full screen; the page
    // stops leaving room for them.
    const window = getCurrentWindow();
    const check = () => void window.isFullscreen().then(handler).catch(() => {});
    check();
    const pending = window.onResized(check);
    let stopped = false;
    void pending.then((un) => {
      if (stopped) un();
    });
    return () => {
      stopped = true;
      void pending.then((un) => un());
    };
  },

  dragWindow(): void {
    void getCurrentWindow().startDragging().catch((error) => console.warn("Could not move the window:", error));
  },

  titleBarDoubleClick(): void {
    void invoke("title_bar_double_click").catch((error) => console.warn("Title bar double-click failed:", error));
  },

  async setWindowIcon(visualPack: "classic" | "dimensional"): Promise<void> {
    const asset = visualPack === "dimensional"
      ? "assets/brand/dimensional/app-icon-512.png"
      : "assets/brand/icon-512.png";
    const response = await fetch(`${import.meta.env.BASE_URL}${asset}`);
    if (!response.ok) throw new Error(`Window icon unavailable: ${response.status}`);
    await getCurrentWindow().setIcon(new Uint8Array(await response.arrayBuffer()));
  },

  openWorkspace(cwd: string, trust?: boolean): Promise<BackendInfo> {
    return invoke<BackendInfo>("start_backend", { cwd, trust: trust ?? null });
  },

  reviewWorkspace(cwd: string): Promise<WorkspaceReview> {
    return invoke<WorkspaceReview>("review_workspace", { cwd });
  },

  openOAuthUrl(url: string): Promise<void> {
    return invoke("open_oauth_url", { url });
  },

  async pickWorkspace(): Promise<string | null> {
    const dir = await openDialog({ directory: true, multiple: false, title: "Open a workspace" });
    return typeof dir === "string" ? dir : null;
  },

  async forgetWorkspace(cwd: string): Promise<void> {
    try {
      await invoke("forget_workspace_desktop", { cwd });
    } catch (e) {
      console.warn("forget_workspace_desktop failed:", e);
    }
  },

  async saveFile(suggestedName: string, bytes: Uint8Array<ArrayBuffer>): Promise<SaveOutcome> {
    const extension = suggestedName.split(".").pop() ?? "txt";
    const path = await saveDialog({
      title: "Save",
      defaultPath: suggestedName,
      filters: [{ name: extension.toUpperCase(), extensions: [extension] }],
    });
    if (!path) return { kind: "cancelled" };
    // Raw body: a document's bytes never travel as a JSON number array.
    await invoke("export_file", bytes, { headers: { "vak-save-path": encodeURIComponent(path) } });
    return { kind: "saved", path };
  },

  async openWith(path: string): Promise<void> {
    await invoke("open_workspace_file", { path });
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

  getAutostart(): Promise<boolean> {
    return invoke<boolean>("get_desktop_autostart");
  },

  setAutostart(enabled: boolean): Promise<void> {
    return invoke<void>("set_desktop_autostart", { enabled });
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
