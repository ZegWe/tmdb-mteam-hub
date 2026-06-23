import { resolve } from "node:path";
import vue from "@vitejs/plugin-vue";
import { defineConfig } from "vite-plus";

export default defineConfig({
  root: "frontend",
  plugins: [vue()],
  fmt: {
    ignorePatterns: ["static/**", "target/**", "Cargo.toml"],
  },
  lint: {
    ignorePatterns: ["static/**", "target/**"],
  },
  resolve: {
    alias: {
      "@": resolve(__dirname, "frontend/src"),
    },
  },
  server: {
    host: "0.0.0.0",
    port: 5173,
    proxy: {
      "/api": "http://127.0.0.1:8787",
    },
  },
  preview: {
    host: "0.0.0.0",
    port: 4173,
  },
  build: {
    outDir: "../static",
    emptyOutDir: true,
    sourcemap: false,
  },
});
