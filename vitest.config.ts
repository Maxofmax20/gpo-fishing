/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The Tauri app's own Vite config lives in vite.config.ts. This one exists only so
// `vitest` can resolve the same plugin and aliases the component expects; the
// production build is untouched.
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { "@": "/src" },
  },
  test: {
    environment: "jsdom",
    globals: false,
    setupFiles: ["./src/test-setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
    // The component resolves its artefacts through `api`, which the tests mock, so
    // no test should ever touch the filesystem or the real dataset.
    restoreMocks: true,
  },
});