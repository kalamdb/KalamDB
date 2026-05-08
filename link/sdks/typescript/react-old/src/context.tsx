import { createContext, useContext } from 'react';
import type { KalamDBClient } from '@kalamdb/client';
import type { KalamProviderProps } from './types.js';

const KalamContext = createContext<KalamDBClient | null>(null);

export function KalamProvider({ client, children }: KalamProviderProps) {
  return <KalamContext.Provider value={client}>{children}</KalamContext.Provider>;
}

export function useKalamClient(client?: KalamDBClient): KalamDBClient {
  const contextClient = useContext(KalamContext);
  const resolvedClient = client ?? contextClient;
  if (!resolvedClient) {
    throw new Error('KalamDB client missing. Wrap your app in <KalamProvider client={client}> or pass client explicitly.');
  }

  return resolvedClient;
}