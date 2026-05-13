import { Auth, createClient, type KalamDBClient } from "@kalamdb/client";

let singleton: KalamDBClient | null = null;

export function getClient(): KalamDBClient {
  if (singleton) return singleton;
  const url = new URL("/kdb", window.location.origin).toString();
  const user = (import.meta.env.VITE_KALAM_USER as string | undefined) ?? "root";
  const password = (import.meta.env.VITE_KALAM_PASSWORD as string | undefined) ?? "kalamdb123";
  singleton = createClient({
    url,
    authProvider: async () => Auth.basic(user, password),
    disableCompression: true,
  });
  return singleton;
}
