#!/usr/bin/env node
import 'dotenv/config';
import { readdir, readFile, stat, writeFile } from 'node:fs/promises';
import { watch } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createKalamClient, resolveKalamConnection } from './client.js';
import { decideConflictAction } from './conflicts.js';
import {
  fetchServerSha256,
  guessMimeType,
  markDeleted,
  sha256Hex,
  upsertMetadata,
} from './file-store.js';
import { parseFrontmatter } from './frontmatter.js';

const ROOT = resolve(fileURLToPath(new URL('..', import.meta.url)));

type SyncStateEntry = {
  serverSha256: string | null;
  localSha256: string;
  lastPulledAt: string;
};

type SyncState = Record<string, SyncStateEntry>;

function syncUser(): string {
  return process.env.KALAM_SYNC_USER ?? 'alice';
}

function watchDir(user: string): string {
  return join(ROOT, 'context', user);
}

function statePath(user: string): string {
  return join(watchDir(user), '.kalam-sync-state.json');
}

async function loadState(user: string): Promise<SyncState> {
  try {
    const raw = await readFile(statePath(user), 'utf8');
    return JSON.parse(raw) as SyncState;
  } catch {
    return {};
  }
}

async function saveState(user: string, state: SyncState): Promise<void> {
  await writeFile(statePath(user), `${JSON.stringify(state, null, 2)}\n`, 'utf8');
}

async function listFiles(dir: string, base = dir): Promise<string[]> {
  const entries = await readdir(dir, { withFileTypes: true });
  const files: string[] = [];

  for (const entry of entries) {
    const fullPath = join(dir, entry.name);
    if (entry.name === '.kalam-sync-state.json') {
      continue;
    }

    if (entry.isDirectory()) {
      files.push(...(await listFiles(fullPath, base)));
      continue;
    }

    if (entry.isFile()) {
      files.push(relative(base, fullPath).replaceAll('\\', '/'));
    }
  }

  return files;
}

async function pushFile(
  user: string,
  relativePath: string,
  state: SyncState,
): Promise<SyncState> {
  const connection = resolveKalamConnection({
    ...process.env,
    KALAM_USER: user,
    KALAM_PASSWORD: user === 'alice' ? 'alice123' : user === 'bob' ? 'bob123' : process.env.KALAM_PASSWORD,
  });
  const client = createKalamClient(connection);
  await client.initialize();

  const fullPath = join(watchDir(user), relativePath);
  const bytes = new Uint8Array(await readFile(fullPath));
  const hash = sha256Hex(bytes);
  const saved = state[relativePath];

  if (saved?.localSha256 === hash) {
    await client.disconnect();
    return state;
  }

  const mimeType = guessMimeType(relativePath);
  const serverSha256 = await fetchServerSha256(client, relativePath);
  const decision = decideConflictAction({
    relativePath,
    localBaseSha256: saved?.serverSha256 ?? null,
    serverSha256,
  });

  const frontmatter =
    mimeType === 'text/markdown'
      ? parseFrontmatter(new TextDecoder().decode(bytes))
      : null;

  const targetPath = decision.path;

  await upsertMetadata(client, {
    path: targetPath,
    sha256: hash,
    baseSha256: serverSha256,
    mimeType,
    sizeBytes: bytes.byteLength,
    frontmatter,
    isConflict: decision.kind === 'create-conflict',
    canonicalPath: decision.kind === 'create-conflict' ? decision.canonicalPath : null,
    deleted: false,
    fileBytes: bytes,
  });

  await client.disconnect();

  const next = { ...state };
  next[relativePath] = {
    serverSha256: hash,
    localSha256: hash,
    lastPulledAt: new Date().toISOString(),
  };
  return next;
}

async function removeFile(user: string, relativePath: string, state: SyncState): Promise<SyncState> {
  const connection = resolveKalamConnection({
    ...process.env,
    KALAM_USER: user,
    KALAM_PASSWORD: user === 'alice' ? 'alice123' : user === 'bob' ? 'bob123' : process.env.KALAM_PASSWORD,
  });
  const client = createKalamClient(connection);
  await client.initialize();
  await markDeleted(client, relativePath);
  await client.disconnect();

  const next = { ...state };
  delete next[relativePath];
  return next;
}

async function syncAll(user: string): Promise<void> {
  const dir = watchDir(user);
  let state = await loadState(user);
  const files = await listFiles(dir);

  for (const path of files) {
    state = await pushFile(user, path, state);
  }

  await saveState(user, state);
  console.log(`[sync] synced ${files.length} file(s) for ${user}`);
}

function debounce<T extends (...args: never[]) => void>(fn: T, ms: number): T {
  let timer: NodeJS.Timeout | undefined;
  return ((...args: Parameters<T>) => {
    clearTimeout(timer);
    timer = setTimeout(() => fn(...args), ms);
  }) as T;
}

async function ensureDemoUsers(): Promise<void> {
  const connection = resolveKalamConnection({
    ...process.env,
    KALAM_USER: 'root',
    KALAM_PASSWORD: process.env.KALAM_ROOT_PASSWORD ?? 'kalamdb123',
  });
  const client = createKalamClient(connection);
  await client.initialize();

  for (const [name, password] of [['alice', 'alice123'], ['bob', 'bob123']] as const) {
    try {
      await client.query(`CREATE USER '${name}' WITH PASSWORD '${password}' ROLE 'user'`);
    } catch {
      // User already exists on subsequent kalam dev runs.
    }
  }

  await client.disconnect();
}

async function main(): Promise<void> {
  const user = syncUser();
  const dir = watchDir(user);

  await ensureDemoUsers();
  console.log(`[sync] watching ${dir} as ${user}`);
  await syncAll(user);

  let state = await loadState(user);
  const schedulePush = debounce(async (relativePath: string) => {
    try {
      state = await pushFile(user, relativePath, state);
      await saveState(user, state);
      console.log(`[sync] pushed ${relativePath}`);
    } catch (error) {
      console.error(`[sync] failed to push ${relativePath}:`, error);
    }
  }, 250);

  watch(dir, { recursive: true }, (_event: string, filename: string | null) => {
    if (!filename || filename === '.kalam-sync-state.json') {
      return;
    }

    const relativePath = filename.replaceAll('\\', '/');
    const fullPath = join(dir, relativePath);

    void stat(fullPath)
      .then((info: { isFile: () => boolean }) => {
        if (info.isFile()) {
          return schedulePush(relativePath);
        }
        return undefined;
      })
      .catch(() => removeFile(user, relativePath, state).then(async (next) => {
        state = next;
        await saveState(user, state);
        console.log(`[sync] marked deleted ${relativePath}`);
      }));
  });
}

void main().catch((error) => {
  console.error('[sync] fatal error:', error);
  process.exit(1);
});
