import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const exampleRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

export default async function globalSetup() {
  execFileSync(process.execPath, ['setup.mjs', '--force'], {
    cwd: exampleRoot,
    stdio: 'inherit',
    env: {
      ...process.env,
      KALAMDB_URL: process.env.KALAMDB_URL ?? 'http://127.0.0.1:2900',
      KALAMDB_USER: process.env.KALAMDB_USER ?? 'admin',
      KALAMDB_PASSWORD: process.env.KALAMDB_PASSWORD ?? 'kalamdb123',
    },
  });
}
