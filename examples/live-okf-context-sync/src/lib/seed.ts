import { copyFile, mkdir } from 'node:fs/promises';
import { defaultSeedDir, listFilesRecursive, listSyncFiles, syncFilePath, syncParentDir } from './paths.js';

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
  const seedFiles = await listFilesRecursive(seedDir);
  let created = 0;

  for (const relativePath of seedFiles) {
    const target = syncFilePath(syncDir, relativePath);
    await mkdir(syncParentDir(syncDir, relativePath), { recursive: true });
    await copyFile(syncFilePath(seedDir, relativePath), target);
    created += 1;
  }

  return created;
}
