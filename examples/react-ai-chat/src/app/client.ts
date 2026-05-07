import { Auth, createClient, type KalamDBClient } from '@kalamdb/client';
import { createDemoClient } from './demo-client';

const DEMO_MODE = import.meta.env.VITE_KALAMDB_DEMO_MODE !== 'false';

let singleton: KalamDBClient | null = null;

export function isExampleDemoMode(): boolean {
  return DEMO_MODE;
}

export function getExampleClient(): KalamDBClient {
  if (singleton) {
    return singleton;
  }

  if (DEMO_MODE) {
    singleton = createDemoClient();
    return singleton;
  }

  singleton = createClient({
    url: import.meta.env.VITE_KALAMDB_URL ?? 'http://127.0.0.1:8080',
    authProvider: async () => Auth.basic(
      import.meta.env.VITE_KALAMDB_USER ?? 'admin',
      import.meta.env.VITE_KALAMDB_PASSWORD ?? 'kalamdb123',
    ),
    disableCompression: true,
  });
  return singleton;
}