import path from "path";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  define: {
    "process.env.DRAGGABLE_DEBUG": "false",
  },
  optimizeDeps: {
    rolldownOptions: {
      transform: {
        define: {
          "process.env.DRAGGABLE_DEBUG": "false",
        },
      },
    },
  },
  plugins: [react(), tailwindcss()],
  build: {
    manifest: true,
  },
  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "./src"),
      ...(process.env.VITE_CVEC_PERF_PROFILER === "on" ? { "react-dom/client": "react-dom/profiling" } : {})
    }
  },
  server: {
    port: 1420,
    strictPort: true
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    css: true,
    restoreMocks: true,
  },
  clearScreen: false
});
