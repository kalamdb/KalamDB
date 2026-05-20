import { defineConfig } from '@playwright/test';

const chatTestRoom = process.env.CHAT_TEST_ROOM ?? `playwright-room-${Date.now()}`;
const chatTestPort = Number(process.env.CHAT_TEST_PORT ?? 5174);
const kalamdbUrl = process.env.KALAMDB_URL ?? 'http://127.0.0.1:2900';
const kalamdbUser = process.env.KALAMDB_USER ?? 'admin';
const kalamdbPassword = process.env.KALAMDB_PASSWORD ?? 'kalamdb123';
process.env.CHAT_TEST_ROOM = chatTestRoom;

export default defineConfig({
  testDir: './tests',
  testMatch: '**/*.spec.mjs',
  timeout: 90_000,
  fullyParallel: false,
  use: {
    baseURL: `http://127.0.0.1:${chatTestPort}`,
    headless: true,
  },
  webServer: {
    command: `npm run dev -- --host 127.0.0.1 --port ${chatTestPort} --strictPort`,
    env: {
      ...process.env,
      VITE_CHAT_ROOM: chatTestRoom,
      VITE_KALAMDB_URL: kalamdbUrl,
      VITE_KALAMDB_USER: kalamdbUser,
      VITE_KALAMDB_PASSWORD: kalamdbPassword,
    },
    port: chatTestPort,
    reuseExistingServer: false,
    timeout: 60_000,
  },
});