import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In dev, the Vite dev server proxies /api to the Rust backend on :8080
// so the SPA can call same-origin URLs. In prod the binary serves the
// built bundle from the same origin via rust-embed (M6).
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/api": "http://127.0.0.1:8080",
      "/healthz": "http://127.0.0.1:8080",
      "/readyz": "http://127.0.0.1:8080",
    },
  },
  build: {
    target: "es2022",
    sourcemap: true,
  },
});
