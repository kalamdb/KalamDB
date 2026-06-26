export const KALAM_FILE_UPLOAD = Symbol.for('@kalamdb/orm/fileUpload');

export interface KalamFileUpload {
  [KALAM_FILE_UPLOAD]: true;
  name: string;
  blob: File | Blob;
}

export function kalamFile(name: string, blob: File | Blob): KalamFileUpload {
  return { [KALAM_FILE_UPLOAD]: true, name, blob };
}

export function isKalamFileUpload(value: unknown): value is KalamFileUpload {
  return (
    typeof value === 'object'
    && value !== null
    && (value as KalamFileUpload)[KALAM_FILE_UPLOAD] === true
  );
}

export function rewriteSqlParamsForFileUploads(
  sql: string,
  params: unknown[],
): { sql: string; params: unknown[]; files: Record<string, File | Blob> } {
  const files: Record<string, File | Blob> = {};
  const uploadParamNumbers = new Set<number>();

  for (let index = 0; index < params.length; index++) {
    const param = params[index];
    if (!isKalamFileUpload(param)) {
      continue;
    }
    uploadParamNumbers.add(index + 1);
    files[param.name] = param.blob;
  }

  if (uploadParamNumbers.size === 0) {
    return { sql, params, files: {} };
  }

  const newParams: unknown[] = [];
  const paramRemap = new Map<number, number>();

  for (let index = 0; index < params.length; index++) {
    const paramNumber = index + 1;
    if (uploadParamNumbers.has(paramNumber)) {
      paramRemap.set(paramNumber, 0);
      continue;
    }

    newParams.push(params[index]);
    paramRemap.set(paramNumber, newParams.length);
  }

  let newSql = '';
  for (let index = 0; index < sql.length; index++) {
    const char = sql[index];
    if (char !== '$') {
      newSql += char;
      continue;
    }

    const match = sql.slice(index).match(/^\$(\d+)/);
    if (!match) {
      newSql += char;
      continue;
    }

    const oldNumber = Number.parseInt(match[1], 10);
    const mapped = paramRemap.get(oldNumber);
    if (mapped === 0) {
      const upload = params[oldNumber - 1] as KalamFileUpload;
      newSql += `FILE("${upload.name}")`;
    } else if (mapped !== undefined) {
      newSql += `$${mapped}`;
    } else {
      newSql += match[0];
    }

    index += match[0].length - 1;
  }

  return { sql: newSql, params: newParams, files };
}

export function sqlReferencesFilePlaceholder(sql: string): boolean {
  return /FILE\s*\(\s*"[^"]+"\s*\)/i.test(sql);
}
