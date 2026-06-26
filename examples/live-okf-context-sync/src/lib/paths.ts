import { readdir } from 'node:fs/promises';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const LOCAL_INDEX_DIR = '.index';
export const SYNC_DB_NAME = 'sync.db';

/** Directories that live only on disk and must never sync to KalamDB. */
const LOCAL_ONLY_DIRS = new Set([LOCAL_INDEX_DIR, '.git']);

/** Files that live only on disk and must never sync to KalamDB. */
const LOCAL_ONLY_FILES = new Set([
  '.DS_Store',
  'Thumbs.db',
  SYNC_DB_NAME,
  `${SYNC_DB_NAME}-wal`,
  `${SYNC_DB_NAME}-shm`,
]);

const DEFAULT_SYNC_DIR = 'data';
const ROOT = resolve(fileURLToPath(new URL('../..', import.meta.url)));

type ListFilesOptions = {
  skipEntry?: (name: string, isDirectory: boolean) => boolean;
  includePath?: (relativePath: string) => boolean;
};

export function projectRoot(): string {
  return ROOT;
}

export function defaultSeedDir(): string {
  return join(ROOT, 'seed');
}

export function resolveSyncDir(arg: string | undefined, env: Record<string, string | undefined> = process.env): string {
  const folder = arg?.trim() || env.KALAM_SYNC_DIR || DEFAULT_SYNC_DIR;
  return resolve(ROOT, folder);
}

export function indexDir(syncDir: string): string {
  return join(syncDir, LOCAL_INDEX_DIR);
}

export function syncDbPath(syncDir: string): string {
  return join(indexDir(syncDir), SYNC_DB_NAME);
}

export function syncFilePath(syncDir: string, relativePath: string): string {
  return join(syncDir, relativePath);
}

export function syncParentDir(syncDir: string, relativePath: string): string {
  return dirname(syncFilePath(syncDir, relativePath));
}

export function toRelativeSyncPath(baseDir: string, absolutePath: string): string {
  return relative(baseDir, absolutePath).replaceAll('\\', '/');
}

export function shouldSkipLocalEntry(name: string, isDirectory: boolean): boolean {
  return isDirectory ? LOCAL_ONLY_DIRS.has(name) : LOCAL_ONLY_FILES.has(name);
}

export function isExcludedRelativePath(relativePath: string): boolean {
  const normalized = relativePath.replaceAll('\\', '/');
  const parts = normalized.split('/');
  return parts.some((part) => LOCAL_ONLY_DIRS.has(part) || LOCAL_ONLY_FILES.has(part));
}

/**
 * A path is safe to write/delete on disk only if it is a clean, in-tree
 * relative path. Rejects traversal, local-only dirs, and server staging
 * artifacts such as `tmp/<uuid>-<user>/upload`.
 */
export function isSafeSyncPath(relativePath: string): boolean {
  const normalized = relativePath.replaceAll('\\', '/');
  if (normalized.length === 0) {
    return false;
  }
  if (normalized.startsWith('/') || /^[a-zA-Z]:\//.test(normalized)) {
    return false;
  }
  if (normalized.split('/').some((segment) => segment === '..')) {
    return false;
  }
  if (isExcludedRelativePath(normalized) || normalized === 'tmp' || normalized.startsWith('tmp/')) {
    return false;
  }
  return true;
}

export function isExcludedWatchPath(filename: string): boolean {
  const normalized = filename.replaceAll('\\', '/');
  const parts = normalized.split('/');
  if (parts.length === 0) {
    return true;
  }
  if (LOCAL_ONLY_DIRS.has(parts[0]!) || parts.some((part) => LOCAL_ONLY_DIRS.has(part))) {
    return true;
  }
  const base = parts[parts.length - 1]!;
  return LOCAL_ONLY_FILES.has(base);
}

/** Skip chokidar events for local-only paths under the sync folder. */
export function shouldIgnoreWatchAbsolutePath(syncDir: string, absolutePath: string): boolean {
  const rel = toRelativeSyncPath(syncDir, absolutePath);
  if (rel === '' || rel.startsWith('..')) {
    return true;
  }
  return isExcludedWatchPath(rel) || isExcludedRelativePath(rel);
}

/** Recursively list relative file paths under `dir`. */
export async function listFilesRecursive(
  dir: string,
  base = dir,
  options: ListFilesOptions = {},
): Promise<string[]> {
  let entries;
  try {
    entries = await readdir(dir, { withFileTypes: true });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
      return [];
    }
    throw error;
  }

  const files: string[] = [];

  for (const entry of entries) {
    if (options.skipEntry?.(entry.name, entry.isDirectory())) {
      continue;
    }

    const fullPath = join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await listFilesRecursive(fullPath, base, options)));
      continue;
    }

    if (entry.isFile()) {
      const relativePath = relative(base, fullPath).replaceAll('\\', '/');
      if (!options.includePath || options.includePath(relativePath)) {
        files.push(relativePath);
      }
    }
  }

  return files;
}

export async function listSyncFiles(dir: string, base = dir): Promise<string[]> {
  return listFilesRecursive(dir, base, {
    skipEntry: shouldSkipLocalEntry,
    includePath: isSafeSyncPath,
  });
}
