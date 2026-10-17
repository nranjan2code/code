import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

export default defineConfig({
  plugins: [solid()],
  base: "/admin/",
  server: {
    // Point at an installed local Vak server to review the checkout's admin
    // UI against its real data without stopping the user's running service.
    proxy: process.env.VAK_ADMIN_API_PROXY
      ? {
          "/admin/api": {
            target: process.env.VAK_ADMIN_API_PROXY,
            changeOrigin: false,
          },
          "^/(?!admin(?:/|$)|@vite(?:/|$)|@id(?:/|$)|src(?:/|$)|node_modules(?:/|$)).*": {
            target: process.env.VAK_ADMIN_API_PROXY,
            changeOrigin: false,
          },
        }
      : undefined,
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
  },
});
