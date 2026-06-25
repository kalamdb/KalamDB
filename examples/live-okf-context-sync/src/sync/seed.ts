import { copyFile, mkdir, readdir } from 'node:fs/promises';
import { dirname, join, relative } from 'node:path';
import { defaultSeedDir, listSyncFiles } from '../lib/paths.js';

async function listSeedFiles(seedDir: string, base = seedDir): Promise<string[]> {
  const entries = await readdir(seedDir, { withFileTypes: true });
  const files: string[] = [];

  for (const entry of entries) {
    const fullPath = join(seedDir, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await listSeedFiles(fullPath, base)));
      continue;
    }

    if (entry.isFile()) {
      files.push(relative(base, fullPath).replaceAll('\\', '/'));
    }
  }

  return files;
}

/**
 * Copy everything from `seed/` into the sync folder when there are no user
 * files yet and the server has no rows (first-time bootstrap only).
 */
export async function maybeSeedSyncFolder(
  syncDir: string,
  serverHasFiles: boolean,
  seedDir = defaultSeedDir(),
): Promise<number> {
  const existing = await listSyncFiles(syncDir);
  if (existing.length > 0 || serverHasFiles) {
    return 0;
  }

  await mkdir(syncDir, { recursive: true });
  const seedFiles = await listSeedFiles(seedDir);
  let created = 0;

  for (const relativePath of seedFiles) {
    const target = join(syncDir, relativePath);
    await mkdir(dirname(target), { recursive: true });
    await copyFile(join(seedDir, relativePath), target);
    created += 1;
  }

  return created;
}
