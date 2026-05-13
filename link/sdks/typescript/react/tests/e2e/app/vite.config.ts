import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

const kalamUrl = process.env.VITE_KALAMDB_URL ?? "http://127.0.0.1:2900";

export default defineConfig({
  plugins: [react()],
  server: {
    fs: {
      allow: [path.resolve(__dirname, "../../../..")],
    },
    proxy: {
      "/kdb": {
        target: kalamUrl,
        changeOrigin: true,
        ws: true,
        rewrite: (p) => p.replace(/^\/kdb/, ""),
        configure: (proxy) => {
          proxy.on("proxyReq", (proxyReq) => {
            proxyReq.removeHeader("origin");
            proxyReq.removeHeader("referer");
          });
          proxy.on("proxyReqWs", (proxyReq) => {
            proxyReq.removeHeader("origin");
            proxyReq.removeHeader("referer");
          });
        },
      },
    },
  },
});
