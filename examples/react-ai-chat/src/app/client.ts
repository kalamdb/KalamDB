import { Auth, createClient, type KalamDBClient } from '@kalamdb/client';
import { createDemoClient } from './demo-client';

const DEMO_MODE = import.meta.env.VITE_KALAM_DEMO_MODE === 'true';

let singleton: KalamDBClient | null = null;

function envValue(key: string): string | undefined {
  const value = import.meta.env[key];
  return typeof value === 'string' && value.length > 0 ? value : undefined;
}

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

  const url = envValue('VITE_KALAM_URL') ?? 'http://127.0.0.1:2900';
  const user = envValue('VITE_KALAM_USER') ?? 'root';
  const password = envValue('VITE_KALAM_PASSWORD') ?? 'kalamdb123';

  singleton = createClient({
    url,
    namespace: 'react_ai_chat',
    authProvider: async () => Auth.basic(user, password),
    disableCompression: true,
  });
  return singleton;
}
