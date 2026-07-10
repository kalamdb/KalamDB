#!/usr/bin/env node
/**
 * Regenerates the typed Drizzle/Kalam schema for the chat_demo namespace.
 * Runs on any OS — no bash required.
 */

import { config as loadEnv } from 'dotenv';
import { spawnSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectDir = resolve(__dirname, '..');

loadEnv({ path: resolve(projectDir, '.env.local'), quiet: true });
loadEnv({ path: resolve(projectDir, '.env'), quiet: true });

const ormCli = resolve(projectDir, 'node_modules/@kalamdb/orm/dist/cli.js');
const outFile = resolve(projectDir, 'src/schema.generated.ts');

const url = process.env.KALAMDB_URL ?? 'http://127.0.0.1:2900';
const user = process.env.KALAMDB_USER ?? 'root';
const password = process.env.KALAMDB_PASSWORD ?? 'kalamdb123';

const result = spawnSync(
  process.execPath,
  [
    '--preserve-symlinks',
    '--preserve-symlinks-main',
    ormCli,
    '--url', url,
    '--user', user,
    '--password', password,
    '--namespace', 'chat_demo',
    '--include-system-columns',
    '--out', outFile,
  ],
  { stdio: 'inherit' },
);

if (result.error) {
  console.error('generate-schema failed:', result.error.message);
  process.exit(1);
}
process.exit(result.status ?? 0);
