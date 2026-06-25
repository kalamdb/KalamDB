import { Auth, createClient, type KalamDBClient } from '@kalamdb/client';

export const NAMESPACE = 'okf_sync';
export const TABLE = `${NAMESPACE}.context_files`;

export type KalamConnection = {
  url: string;
  user: string;
  password: string;
};

export function resolveKalamConnection(
  env: Record<string, string | undefined> = process.env,
): KalamConnection {
  return {
    url: env.KALAM_URL ?? env.VITE_KALAM_URL ?? 'http://127.0.0.1:2900',
    user: env.KALAM_USER ?? env.VITE_KALAM_USER ?? 'alice',
    password: env.KALAM_PASSWORD ?? env.VITE_KALAM_PASSWORD ?? 'alice123',
  };
}

export function createKalamClient(connection: KalamConnection): KalamDBClient {
  return createClient({
    url: connection.url,
    namespace: NAMESPACE,
    authProvider: async () => Auth.basic(connection.user, connection.password),
    disableCompression: true,
  });
}

export const LIVE_FILES_SQL = [
  'SELECT path, sha256, mime_type, size_bytes, is_conflict, canonical_path, deleted, updated_at',
  `FROM ${TABLE}`,
  'ORDER BY updated_at DESC',
].join(' ');
