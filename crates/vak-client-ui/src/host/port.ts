// The host port (docs/design/48-web-client.md §3.1).
//
// One client, three hosts: the Tauri desktop shell, a browser on loopback,
// and a browser against a headless box. Everything that differs between
// them lives behind this interface and nowhere else — no component
// imports `@tauri-apps/*`, and none sniffs for `window.__TAURI__`.
//
// The rule that keeps this honest: a capability the host cannot provide is
// a DESIGN DECISION, not an error state. `can()` is checked so the UI
// renders differently rather than brokenly — the dock has no Terminal tab
// on a host without one, instead of a disabled tab that explains nothing.

import type { BackendInfo, WorkspaceReview } from "../types";

export type HostKind = "desktop" | "web";

export type HostFeature =
  /** OS file pickers (vs. a server-side directory browser). */
  | "native-dialogs"
  /** A real PTY. Off by default on a remote host: a shell over HTTP is
   *  remote code execution, and unlike every other effect in the product
   *  it is not mediated by the permission engine. */
  | "terminal"
  /** Can open arbitrary folders as workspaces. */
  | "multi-workspace"
  /** OS notification centre (vs. the Web Notifications API). */
  | "system-notifications"
  /** A menu-bar/tray presence and a background service lifecycle. */
  | "tray"
  /** A user microphone can be captured by the host. */
  | "microphone"
  /** Hand a workspace document to the application the operating system
   *  associates with it ("Open with…"). Desktop only: a browser cannot
   *  launch an application on the machine that holds the file. */
  | "open-with";

/** Where a saved document ended up, so callers can word their own notice. */
export type SaveOutcome =
  | { kind: "saved"; path: string }
  | { kind: "downloaded" }
  | { kind: "cancelled" };

/** Bidirectional byte transport for one terminal session. */
export interface TerminalTransport {
  /** Raw bytes from the shell. */
  onData(handler: (bytes: Uint8Array) => void): void;
  /** Called when the shell exits on its own. */
  onExit(handler: () => void): void;
  write(data: string): void;
  resize(cols: number, rows: number): void;
  /** Ends the shell and releases the transport. Always safe to call twice. */
  close(): void;
}

export interface Host {
  readonly kind: HostKind;

  /** Capability probe. Never infer a capability from `kind`. */
  can(feature: HostFeature): boolean;

  /** Backend identity: base URL, auth channel, cwd, recents, boot error. */
  info(): Promise<BackendInfo>;
  /** Host-level changes (workspace opened, boot failed). Returns unsubscribe. */
  onInfoChanged(handler: (info: BackendInfo) => void): () => void;
  /** Whether the window is in full screen, now and on each change. A host
   *  whose window chrome never overlays the page reports nothing. Returns
   *  unsubscribe. */
  onFullscreenChange(handler: (fullscreen: boolean) => void): () => void;

  /** Change the running window's icon where the host supports it. This does
   * not change the icon installed by the operating system. */
  setWindowIcon?(visualPack: "classic" | "dimensional"): Promise<void>;

  /** Open `cwd` as the active workspace. `trust` is the operator's answer
   *  when they have just been asked, and undefined when nobody is being
   *  asked — in which case the decision already on record governs. */
  openWorkspace(cwd: string, trust?: boolean): Promise<BackendInfo>;
  /** What a folder would ask for, before anything opens it. */
  reviewWorkspace(cwd: string): Promise<WorkspaceReview>;
  /** Choose a folder: a native dialog, or a server-side browser. `null`
   *  when the operator cancelled. */
  pickWorkspace(): Promise<string | null>;
  /** Stop listing a workspace from host recents. */
  forgetWorkspace?(cwd: string): Promise<void>;

  /** Get bytes to the operator: a native save dialog, or a download. */
  saveFile(suggestedName: string, bytes: Uint8Array<ArrayBuffer>, mime: string): Promise<SaveOutcome>;

  /** Open a workspace document in its associated application. Present only
   *  where `can("open-with")`; the host refuses anything outside the
   *  workspace, anything not an Office document, and macro-enabled files. */
  openWith?(path: string): Promise<void>;

  /** `route` is where a click should land, as a hash route. Hosts that
   *  cannot make a notification clickable simply ignore it. */
  notify(title: string, body: string, route?: string): Promise<void>;

  /** Open the operations console. Same-origin on the web; a real browser
   *  launch from the desktop shell. */
  openAdmin(route: string): void;

  /** Autostart on machine login/boot (desktop shell with tray only). */
  getAutostart?(): Promise<boolean>;
  setAutostart?(enabled: boolean): Promise<void>;

  /** `null` when this host has no terminal — see `can("terminal")`. */
  terminal(cwd: string): Promise<TerminalTransport | null>;

  // ---- Auth ---------------------------------------------------------
  //
  // Present only where signing in is a thing that happens. The desktop
  // shell holds its embedded server's token already and never shows a
  // login screen, so it implements none of these — and the login view is
  // rendered only when `authenticate` exists, rather than being gated on
  // `kind === "web"`.

  /** Exchange a bearer token for a session. */
  authenticate?(token: string): Promise<void>;
  /** Whether this client already holds a valid session. */
  sessionStatus?(): Promise<{ authenticated: boolean; expires_at?: string | null }>;
  /** End the session. */
  logout?(): Promise<void>;
}

/** Which BUILD produced this bundle. Deliberately not `HostKind`: the
 *  build targets are named for their toolchain ("tauri" is a shell), the
 *  runtime kinds for what the user is looking at ("desktop" is a window).
 *  Collapsing the two reads fine until a second desktop shell exists. */
export type BuildHost = "tauri" | "web";

declare const __VAK_BUILD_HOST__: BuildHost | undefined;

/** Which host this bundle was built for. Settled at build time by
 *  vite.config.ts, not sniffed at runtime — sniffing for `window.__TAURI__`
 *  races the shell's own injection and fails intermittently on cold start. */
export function buildHost(): BuildHost {
  return typeof __VAK_BUILD_HOST__ === "string" ? __VAK_BUILD_HOST__ : "tauri";
}
