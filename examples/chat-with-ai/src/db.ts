import { Auth, createClient } from '@kalamdb/client';
import { kalamDriver } from '@kalamdb/orm';
import { drizzle } from 'drizzle-orm/pg-proxy';

function viteEnv(name: string, fallback: string): string {
  const value = (import.meta.env as Record<string, string | undefined>)[name];
  return typeof value === 'string' && value.length > 0 ? value : fallback;
}

export const ROOM = viteEnv('VITE_CHAT_ROOM', 'main');
export const CHAT_USERNAME = viteEnv('VITE_KALAM_USER', viteEnv('VITE_KALAMDB_USER', 'root'));
export const CHAT_PASSWORD = viteEnv('VITE_KALAM_PASSWORD', viteEnv('VITE_KALAMDB_PASSWORD', 'kalamdb123'));

export function membershipId(userId: string, roomId: string): string {
  return `${userId}:${roomId}`;
}

export const client = createClient({
  url: viteEnv('VITE_KALAM_URL', viteEnv('VITE_KALAMDB_URL', 'http://127.0.0.1:2900')),
  authProvider: async () => Auth.basic(CHAT_USERNAME, CHAT_PASSWORD),
  disableCompression: true,
});

export const db = drizzle(kalamDriver(client));
