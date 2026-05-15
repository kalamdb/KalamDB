import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import path from "node:path";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  test: {
    include: ["tests/component/**/*.test.{ts,tsx}"],
    environment: "jsdom",
    setupFiles: ["./tests/component/setup.ts"],
    globals: true,
    css: false,
  },
});
