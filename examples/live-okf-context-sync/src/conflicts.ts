export type ConflictDecision =
  | { kind: 'update-canonical'; path: string }
  | { kind: 'create-conflict'; path: string; canonicalPath: string };

export function conflictCopyPath(canonicalPath: string, at = new Date()): string {
  const stamp = at.toISOString().replace(/[:.]/g, '-');
  const dot = canonicalPath.lastIndexOf('.');
  if (dot === -1) {
    return `${canonicalPath}.conflict-${stamp}`;
  }

  const base = canonicalPath.slice(0, dot);
  const ext = canonicalPath.slice(dot);
  return `${base}.conflict-${stamp}${ext}`;
}

export function decideConflictAction(input: {
  relativePath: string;
  localBaseSha256: string | null;
  serverSha256: string | null;
}): ConflictDecision {
  const { relativePath, localBaseSha256, serverSha256 } = input;

  if (!serverSha256 || !localBaseSha256 || serverSha256 === localBaseSha256) {
    return { kind: 'update-canonical', path: relativePath };
  }

  return {
    kind: 'create-conflict',
    path: conflictCopyPath(relativePath),
    canonicalPath: relativePath,
  };
}
