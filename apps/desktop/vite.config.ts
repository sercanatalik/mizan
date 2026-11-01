/// <reference types="vitest/config" />
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Tauri expects a fixed port and does not need the screen-clearing behavior.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: { target: "safari16", outDir: "dist" },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["src/test/setup.ts"],
    css: false,
  },
});
