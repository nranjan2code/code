import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

export default defineConfig({
  plugins: [solid()],
  base: "/admin/",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
  },
});
