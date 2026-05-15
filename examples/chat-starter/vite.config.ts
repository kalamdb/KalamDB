import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

// Browser-side env vars only — secrets must never appear in import.meta.env.
// KALAMDB_URL is needed at proxy-config time; it's not bundled into the app.
const kalamUrl = process.env.KALAMDB_URL ?? "http://127.0.0.1:8080";
const apiUrl = process.env.API_URL ?? "http://127.0.0.1:3001";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    fs: {
      // The starter consumes @kalamdb/* via file: links to ../../link/sdks/...
      // — allow Vite to serve those workspace files in dev. Not a security
      // boundary; production builds via vite build don't read source files.
      allow: [path.resolve(__dirname, "../..")],
    },
    proxy: {
      // Browser → backend (token issuance, future user APIs).
      "/api": { target: apiUrl, changeOrigin: true },
      // Browser → KalamDB (live queries, SQL). The browser only reaches this
      // path after fetching a token from /api/auth/token; the proxy itself
      // does not attach credentials.
      "/kdb": {
        target: kalamUrl,
        changeOrigin: true,
        ws: true,
        rewrite: (p) => p.replace(/^\/kdb/, ""),
      },
    },
  },
});
