// The single place the active host is chosen (docs/design/48-web-client.md).
//
// Import `host` from here and nothing else — no component should know
// which implementation it got, only what `host.can(...)` answers.
//
// `#host-impl` is resolved by vite.config.ts to `./tauri.ts` or `./web.ts`
// depending on VAK_HOST, and by tsconfig `paths` to the Tauri one for
// typechecking (the two satisfy the same `Host` interface, so either types
// the whole tree correctly). A build-time alias rather than a runtime
// branch, so `@tauri-apps/*` is never even reachable from the web bundle:
// a runtime `if` would still pull the whole Tauri API in as a dependency
// of the module graph, and the web build would ship (and try to
// initialize) an IPC layer that has nothing on the other end.

import { activeHost } from "#host-impl";
import type { Host } from "./port";

export const host: Host = activeHost;

export type { Host, HostFeature, SaveOutcome, TerminalTransport, HostKind, BuildHost } from "./port";
export { buildHost } from "./port";
