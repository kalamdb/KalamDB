import type { Table } from 'drizzle-orm';
import type {
  DrizzleLiveQueryOptions,
  LiveQueriesDefinition,
  RawSqlLiveQueryOptions,
} from '../types.js';

const KALAM_TABLE_CONFIG = Symbol.for('kalamdb.orm.tableConfig');
const DRIZZLE_NAME = Symbol.for('drizzle:Name');
const DRIZZLE_BASE_NAME = Symbol.for('drizzle:BaseName');
const UNIT_SEPARATOR = '';

/**
 * Resolve the qualified KalamDB table name (`schema.table`) for a Drizzle
 * `Table`. Prefers the `@kalamdb/orm` config (set by `kTable.user(...)`)
 * because it preserves the namespace; falls back to Drizzle's internal name
 * symbols for raw `pgTable` callers.
 */
export function tableName(table: Table): string {
  const obj = table as unknown as Record<PropertyKey, unknown>;
  const kalam = obj[KALAM_TABLE_CONFIG] as { qualifiedName?: string } | undefined;
  if (kalam?.qualifiedName) return kalam.qualifiedName;
  const drizzle = obj[DRIZZLE_NAME] ?? obj[DRIZZLE_BASE_NAME];
  if (typeof drizzle === 'string' && drizzle.length > 0) return drizzle;
  throw new Error('Unable to resolve KalamDB table name from the provided table.');
}

function getKeyName(getKey: unknown): string {
  if (typeof getKey === 'function') return 'fn';
  if (Array.isArray(getKey)) return getKey.join(',');
  return String(getKey ?? '');
}

function liveDepsKey(deps: readonly unknown[] | undefined): string {
  return deps?.map((value) => String(value)).join(UNIT_SEPARATOR) ?? '';
}

/**
 * Build a stable cache key for a single `useLiveQuery` invocation. The
 * subscription is recreated when this value changes. Inline `where` / `orderBy`
 * arrows do NOT participate by design — callers opt into resubscribe by
 * threading their input into `deps`, just like React Query / SWR.
 */
export function liveQuerySignature(
  options: DrizzleLiveQueryOptions<Table, unknown> | RawSqlLiveQueryOptions<unknown>,
): string {
  const parts: Array<string | number> = [];
  if ('query' in options && typeof options.query === 'string') {
    parts.push('sql', options.query);
  } else if ('table' in options) {
    parts.push('drizzle', tableName(options.table));
  }
  parts.push(options.limit ?? '');
  parts.push(getKeyName(options.getKey));
  parts.push(liveDepsKey(options.deps));
  return parts.join('|');
}

/** Stable cache key for `useLiveQueries`, computed across all named queries. */
export function liveQueriesSignature(
  queries: LiveQueriesDefinition,
  deps: readonly unknown[] | undefined,
): string {
  const perQuery = Object.entries(queries).map(([name, definition]) =>
    [
      name,
      tableName(definition.table),
      definition.limit ?? '',
      getKeyName(definition.getKey),
      liveDepsKey(definition.deps),
    ].join(':'),
  );
  return [...perQuery, liveDepsKey(deps)].join('|');
}
