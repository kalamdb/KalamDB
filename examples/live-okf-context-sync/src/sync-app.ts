#!/usr/bin/env node
import 'dotenv/config';
import { pathToFileURL } from 'node:url';
import { resolveKalamConnection } from './db/client.js';
import { resolveSyncDir } from './lib/paths.js';
import { FolderSyncApp } from './sync-engine.js';

export { FolderSyncApp, type FolderSyncOptions } from './sync-engine.js';

export async function main(
  argv: string[] = process.argv,
  env: NodeJS.ProcessEnv = process.env,
): Promise<void> {
  const app = new FolderSyncApp({
    syncDir: resolveSyncDir(argv[2], env),
    connection: resolveKalamConnection(env),
  });
  await app.start();
}

const isMainModule = process.argv[1] !== undefined
  && import.meta.url === pathToFileURL(process.argv[1]).href;

if (isMainModule) {
  void main().catch((error) => {
    console.error('[sync] fatal error:', error);
    process.exit(1);
  });
}
