import { createClient, Auth } from '@kalamdb/client';

export const URL = process.env.KALAMDB_TEST_URL || 'http://localhost:2900';
export const USER = process.env.KALAMDB_TEST_USER || 'admin';
export const PASS = process.env.KALAMDB_TEST_PASSWORD || 'kalamdb123';

export function requirePassword() {
  if (!PASS) {
    console.error('KALAMDB_TEST_PASSWORD is required. Run a KalamDB server and set the env var:');
    console.error('  KALAMDB_TEST_PASSWORD=<password> node --test tests/<test-file>');
    process.exit(1);
  }
}

export function createTestClient() {
  return createClient({
    url: URL,
    authProvider: async () => Auth.basic(USER, PASS),
  });
}
