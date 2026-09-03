import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

// One client, two hosts (docs/design/48-web-client.md).
//
// The SAME source tree builds twice:
//
//   VAK_HOST=tauri (default) → dist/      base "./"     loaded from the
//                                                       Tauri shell's own
//                                                       asset protocol
//   VAK_HOST=web             → dist-web/  base "/app/"  embedded into
//                                                       vak-server and
//                                                       served under /app
//
// Two outputs rather than one because the base path is baked into every
// emitted asset URL, and a bundle built for one host is wrong on the other.
// The host adapter itself (src/host/*) is chosen at RUNTIME, not build
// time — this only settles where assets live.
const web = process.env.VAK_HOST === "web";

export default defineConfig({
  plugins: [solid()],
  clearScreen: false,
  base: web ? "/app/" : "./",
  resolve: {
    alias: {
      // src/host/index.ts imports this; only the selected implementation
      // ever enters the module graph, so the web bundle never carries
      // (or tries to initialize) the Tauri IPC layer.
      "#host-impl": new URL(
        web ? "./src/host/web.ts" : "./src/host/tauri.ts",
        import.meta.url,
      ).pathname,
    },
  },
  server: { port: 1420, strictPort: true },
  build: {
    target: "esnext",
    outDir: web ? "dist-web" : "dist",
    emptyOutDir: true,
  },
  define: {
    // Lets the client assert its host at startup rather than sniffing for
    // `window.__TAURI__`, which is a race against the shell's injection.
    __VAK_BUILD_HOST__: JSON.stringify(web ? "web" : "tauri"),
  },
});
