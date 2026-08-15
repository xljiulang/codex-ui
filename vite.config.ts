/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2021",
    outDir: "dist",
    assetsInlineLimit: 0,
  },
  test: {
    environment: "happy-dom",
    setupFiles: ["./vitest.setup.ts"],
    include: ["src/**/*.spec.ts"],
    coverage: {
      provider: "v8",
      all: true,
      include: ["src/**/*.{ts,vue}"],
      exclude: [
        "src/main.ts",
        "src/App.vue",
        "src/vite-env.d.ts",
        "src/lib/markdown.worker.ts",
        "src/lib/types.ts",
        "src/**/*.spec.ts",
      ],
      reporter: ["text", "html", "lcov"],
      reportsDirectory: "coverage",
      // 基线（2026-08-15）：lines 87.34 / functions 84.3 / statements 84.01 / branches 77.24
      // 规则：基线 -5pp，向下取整到 5%
      thresholds: {
        lines: 80,
        functions: 75,
        statements: 75,
        branches: 70,
      },
    },
  },
});
