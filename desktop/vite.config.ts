import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig(({ command }) => {
  // Block 42 "production build absence": a standard production build gets
  // a compile-time `false`, allowing Rollup to remove the LabScreen dynamic
  // import and every Lab-only client wrapper reachable only from it. Dev
  // serves include the screen so `tauri dev --features lab-mode` remains
  // useful. A packaged Lab build must opt in explicitly through
  // `npm run build:lab` / `npm run tauri:lab:build`.
  const includeLabFrontend = command === "serve" || process.env.SILENT_DISCO_LAB_FRONTEND === "1";

  return {
    plugins: [react(), tailwindcss()],
    define: {
      __LAB_FRONTEND_INCLUDED__: JSON.stringify(includeLabFrontend),
    },
    clearScreen: false,
    server: {
      host: "127.0.0.1",
      port: 1420,
      strictPort: true,
    },
    test: {
      environment: "jsdom",
      setupFiles: ["./src/test/setup.ts"],
      css: true,
      restoreMocks: true,
    },
  };
});
