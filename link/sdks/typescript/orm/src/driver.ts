import type { RemoteCallback } from 'drizzle-orm/pg-proxy';
import type { KalamDBClient } from '@kalamdb/client';
import {
  rewriteSqlParamsForFileUploads,
  sqlReferencesFilePlaceholder,
} from './file-upload.js';
import { stripQuotedIdentifiers } from './query-normalize.js';

type QueryClient = Pick<KalamDBClient, 'query'> & Partial<Pick<KalamDBClient, 'queryWithFiles'>>;
type ColumnNormalizer = (value: unknown) => unknown;

export function toMilliseconds(value: number): number {
  if (value > 1e15) return Math.floor(value / 1000);
  if (value > 1e12) return value;
  if (value > 1e9) return value * 1000;
  return value;
}

function toDrizzleTimestampDriverValue(date: Date): string {
  return date.toISOString().replace(/Z$/, '');
}

function toDate32String(days: number): string {
  return new Date(Math.trunc(days) * 86_400_000).toISOString().slice(0, 10);
}

function toTimeStringFromMicros(input: number): string {
  const microsPerDay = 86_400_000_000;
  let micros = Math.trunc(input) % microsPerDay;
  if (micros < 0) micros += microsPerDay;

  const hours = Math.floor(micros / 3_600_000_000);
  micros %= 3_600_000_000;
  const minutes = Math.floor(micros / 60_000_000);
  micros %= 60_000_000;
  const seconds = Math.floor(micros / 1_000_000);
  const fractionalMicros = micros % 1_000_000;

  const base = [hours, minutes, seconds].map((part) => String(part).padStart(2, '0')).join(':');
  return fractionalMicros === 0
    ? base
    : `${base}.${String(fractionalMicros).padStart(6, '0')}`;
}

export function normalizeTemporalValue(value: unknown): unknown {
  if (value === null || value === undefined) return value;

  if (value instanceof Date) {
    return toDrizzleTimestampDriverValue(value);
  }

  if (typeof value === 'number') {
    const date = new Date(toMilliseconds(value));
    return Number.isNaN(date.getTime()) ? String(value) : toDrizzleTimestampDriverValue(date);
  }

  if (typeof value === 'string') {
    const trimmed = value.trim();
    if (/^-?\d+(\.\d+)?$/.test(trimmed)) {
      const num = Number(trimmed);
      if (!Number.isNaN(num)) {
        const date = new Date(toMilliseconds(num));
        if (!Number.isNaN(date.getTime())) return toDrizzleTimestampDriverValue(date);
      }
    }

    if (/([zZ]|[+-]\d{2}:?\d{2})$/.test(trimmed)) {
      const parsed = new Date(trimmed);
      if (!Number.isNaN(parsed.getTime())) {
        return toDrizzleTimestampDriverValue(parsed);
      }
    }
  }

  return value;
}

export function normalizeDateValue(value: unknown): unknown {
  if (value === null || value === undefined) return value;
  if (value instanceof Date) return value.toISOString().slice(0, 10);
  if (typeof value === 'number') return toDate32String(value);
  if (typeof value === 'string') {
    const trimmed = value.trim();
    if (/^-?\d+$/.test(trimmed)) return toDate32String(Number(trimmed));
    if (/^\d{4}-\d{2}-\d{2}/.test(trimmed)) return trimmed.slice(0, 10);
  }
  return value;
}

export function normalizeTimeValue(value: unknown): unknown {
  if (value === null || value === undefined) return value;
  if (typeof value === 'number') return toTimeStringFromMicros(value);
  if (typeof value === 'string') {
    const trimmed = value.trim();
    if (/^-?\d+$/.test(trimmed)) return toTimeStringFromMicros(Number(trimmed));
    if (/^\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?$/.test(trimmed)) return trimmed;
  }
  return value;
}

function normalizerForSchemaType(field: { data_type?: string; dataType?: string; type?: string }): ColumnNormalizer | undefined {
  const rawType = String(field.data_type ?? field.dataType ?? field.type ?? '').toLowerCase();
  if (rawType.includes('timestamp') || rawType === 'datetime') return normalizeTemporalValue;
  if (rawType === 'date' || rawType.includes('date32') || rawType.includes('date64')) return normalizeDateValue;
  if (rawType === 'time' || rawType.includes('time64') || rawType.includes('time32')) return normalizeTimeValue;
  return undefined;
}

export function stripDefaults(sql: string, params: unknown[]): { sql: string; params: unknown[] } {
  if (/on conflict/i.test(sql)) {
    return { sql, params };
  }

  const match = sql.match(/^(INSERT\s+INTO\s+\S+)\s*\(([^)]+)\)\s*VALUES\s*/i);
  if (!match) return { sql, params };

  const prefix = match[1];
  const columns = match[2].split(',').map((c) => c.trim());
  const valuesSql = sql.slice(match[0].length);

  const valueGroups: string[][] = [];
  const remaining = valuesSql.trim();
  const groupRegex = /\(([^)]+)\)/g;
  let groupMatch;
  while ((groupMatch = groupRegex.exec(remaining)) !== null) {
    valueGroups.push(groupMatch[1].split(',').map((v) => v.trim()));
  }
  if (valueGroups.length === 0) return { sql, params };

  const firstGroup = valueGroups[0];
  const keepIndices: number[] = [];
  for (let i = 0; i < firstGroup.length; i++) {
    if (firstGroup[i].toUpperCase() !== 'DEFAULT') keepIndices.push(i);
  }
  if (keepIndices.length === columns.length) return { sql, params };

  const newColumns = keepIndices.map((i) => columns[i]);
  const newParams: unknown[] = [];
  const newValueGroups = valueGroups.map((group) => {
    const vals = keepIndices.map((i) => {
      const val = group[i];
      const paramMatch = val.match(/^\$(\d+)$/);
      if (paramMatch) {
        newParams.push(params[parseInt(paramMatch[1]) - 1]);
        return `$${newParams.length}`;
      }
      return val;
    });
    return `(${vals.join(', ')})`;
  });

  return {
    sql: `${prefix} (${newColumns.join(', ')}) VALUES ${newValueGroups.join(', ')}`,
    params: newParams,
  };
}

export function kalamDriver(client: QueryClient): RemoteCallback {
  return async (sql, params, method) => {
    let cleanSql = stripQuotedIdentifiers(sql);
    const stripped = stripDefaults(cleanSql, params);
    cleanSql = stripped.sql;

    const rewritten = rewriteSqlParamsForFileUploads(cleanSql, stripped.params);
    cleanSql = rewritten.sql;
    const queryParams = rewritten.params;

    let response;
    if (Object.keys(rewritten.files).length > 0) {
      if (!client.queryWithFiles) {
        throw new Error(
          'kalamDriver() received a FILE upload but the client has no queryWithFiles(); pass a KalamDBClient',
        );
      }
      response = await client.queryWithFiles(cleanSql, rewritten.files, queryParams);
    } else if (sqlReferencesFilePlaceholder(cleanSql)) {
      throw new Error(
        'SQL references FILE("...") but no upload bytes were provided; pass kalamFile(name, blob) in .values()/.set()',
      );
    } else {
      response = await client.query(cleanSql, queryParams);
    }

    if (method === 'execute') return { rows: [] };
    const result = response.results?.[0];
    const schema = (result?.schema as Array<{ name: string; data_type?: string; dataType?: string; type?: string }> | undefined) ?? [];
    const columns = schema.map((field) => field.name);
    const columnNormalizers = new Map(
      schema
        .map((field) => [field.name, normalizerForSchemaType(field)] as const)
        .filter((entry): entry is readonly [string, ColumnNormalizer] => Boolean(entry[1])),
    );
    const rows = (result?.named_rows as Record<string, unknown>[] ?? []).map(
      (row) =>
        columns.map((col) => {
          const normalizer = columnNormalizers.get(col);
          return normalizer ? normalizer(row[col]) : row[col];
        }),
    );
    return { rows };
  };
}
