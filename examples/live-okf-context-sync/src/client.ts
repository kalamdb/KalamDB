import { Auth, createClient, type KalamDBClient } from '@kalamdb/client';
import { configureKalamOrm, kalamDriver } from '@kalamdb/orm';
import { drizzle } from 'drizzle-orm/pg-proxy';

export const NAMESPACE = 'okf_sync';
export const TABLE = `${NAMESPACE}.context_files`;

// The generated schema uses unqualified table names (single-namespace codegen),
// so resolve them through the okf_sync namespace once at module load.
configureKalamOrm({ namespace: NAMESPACE });

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

/**
 * Drizzle database bound to a KalamDB connection. Use it for typed reads,
 * updates, and the live table subscription. File byte uploads still go through
 * the raw client (`queryWithFiles`) because the ORM driver speaks SQL, not
 * multipart uploads.
 */
export function createDb(client: KalamDBClient) {
  return drizzle(kalamDriver(client));
}

export type KalamDb = ReturnType<typeof createDb>;
