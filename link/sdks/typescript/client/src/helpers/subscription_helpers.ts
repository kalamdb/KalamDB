import { KalamCellValue, wrapRowMap } from '../cell_value.js';
import type { RowData } from '../cell_value.js';
import { VersionId } from '../version_id.js';
import type {
  LiveEventsOptions,
  LiveOptions,
  LiveStreamOptions,
  ServerMessage,
  SubscriptionInfo,
} from '../types.js';

export function normalizeLiveStreamOptions(
  options?: LiveStreamOptions,
): { batch_size?: number; last_rows?: number; from?: string; auto_fetch_batches?: boolean } | undefined {
  if (!options) {
    return undefined;
  }

  const normalized: { batch_size?: number; last_rows?: number; from?: string; auto_fetch_batches?: boolean } = {};

  if (options.batchSize !== undefined) {
    normalized.batch_size = options.batchSize;
  }

  if (options.lastRows !== undefined) {
    normalized.last_rows = options.lastRows;
  }

  if (options.from !== undefined) {
    normalized.from = options.from instanceof VersionId ? options.from.toString() : VersionId.from(options.from).toString();
  }

  if (options.autoFetchBatches !== undefined) {
    normalized.auto_fetch_batches = options.autoFetchBatches;
  }

  return Object.keys(normalized).length > 0 ? normalized : undefined;
}

export type NormalizedSubscriptionEvent = ServerMessage & {
  rows?: RowData[];
  old_values?: RowData[];
};

export type LiveRowsWasmEvent = {
  type: 'rows' | 'error';
  subscription_id: string;
  rows?: RowData[];
  code?: string;
  message?: string;
  last_seq_id?: VersionId;
};

export interface LocalSubscriptionMetadata {
  tableName: string;
  createdAtMs: number;
}

interface WasmSubscriptionSnapshot {
  id: string;
  query?: string;
  lastSeqId?: string;
  closed?: boolean;
}

interface ParsedWasmSubscriptions {
  parsed: boolean;
  subscriptions: WasmSubscriptionSnapshot[];
}

export function trackSubscriptionMetadata(
  metadata: Map<string, LocalSubscriptionMetadata>,
  subscriptionId: string,
  tableName: string,
): void {
  metadata.set(subscriptionId, {
    tableName,
    createdAtMs: Date.now(),
  });
}

export function normalizeSubscriptionEvent(event: ServerMessage): NormalizedSubscriptionEvent {
  const normalized = { ...event } as NormalizedSubscriptionEvent;

  if ('rows' in normalized) {
    normalized.rows = wrapSubscriptionRows(normalized.rows);
  }
  if ('old_values' in normalized) {
    normalized.old_values = wrapSubscriptionRows(normalized.old_values);
  }

  return normalized;
}

export function lastVersionIdFromSubscriptionEvent(event: NormalizedSubscriptionEvent): VersionId | undefined {
  if ('batch_control' in event) {
    const batchControl = event.batch_control as { last_seq_id?: string | number | VersionId | null } | undefined;
    const batchVersionId = normalizeWireVersionId(batchControl?.last_seq_id);
    if (batchVersionId) {
      return batchVersionId;
    }
  }

  return maxVersionIdFromRows(event.rows) ?? maxVersionIdFromRows(event.old_values);
}

function maxVersionIdFromRows(rows: RowData[] | undefined): VersionId | undefined {
  let max: VersionId | undefined;
  for (const row of rows ?? []) {
    const cell = row._seq;
    const seqId = cell instanceof KalamCellValue
      ? cell.asVersionId()
      : normalizeWireVersionId(cell as string | number | VersionId | null | undefined);
    if (seqId && (!max || seqId.compareTo(max) > 0)) {
      max = seqId;
    }
  }

  return max;
}

export function normalizeLiveRowsWasmEvent(event: {
  type: 'rows' | 'error';
  subscription_id: string;
  rows?: unknown;
  code?: string;
  message?: string;
  last_seq_id?: string | number | VersionId | null;
}): LiveRowsWasmEvent {
  const lastSeqId = normalizeWireVersionId(event.last_seq_id);

  return {
    type: event.type,
    subscription_id: event.subscription_id,
    ...(event.code !== undefined ? { code: event.code } : {}),
    ...(event.message !== undefined ? { message: event.message } : {}),
    rows: wrapSubscriptionRows(event.rows),
    ...(lastSeqId ? { last_seq_id: lastSeqId } : {}),
  };
}

export function normalizeLiveKeyColumns<T>(
  options: LiveOptions<T>,
): string[] | undefined {
  const declaredColumns = typeof options.getKey === 'function'
    ? undefined
    : options.getKey;

  if (declaredColumns === undefined) {
    return undefined;
  }

  const columns = Array.isArray(declaredColumns) ? declaredColumns : [declaredColumns];
  const normalized = columns
    .map((column) => column.trim())
    .filter((column, index, values) => column.length > 0 && values.indexOf(column) === index);

  return normalized.length > 0 ? normalized : undefined;
}

function normalizeWireVersionId(value: string | number | VersionId | null | undefined): VersionId | undefined {
  if (value === null || value === undefined) {
    return undefined;
  }

  try {
    return value instanceof VersionId ? value : VersionId.from(value);
  } catch {
    return undefined;
  }
}

export function normalizeLiveOptions<T>(
  options: LiveOptions<T>,
): {
  limit?: number;
  key_columns?: string[];
  subscription_options?: { batch_size?: number; last_rows?: number; from?: string; auto_fetch_batches?: boolean };
} | undefined {
  const keyColumns = normalizeLiveKeyColumns(options);
  const streamOptions = normalizeLiveStreamOptions(options);
  const normalized: {
    limit?: number;
    key_columns?: string[];
    subscription_options?: { batch_size?: number; last_rows?: number; from?: string; auto_fetch_batches?: boolean };
  } = {};

  if (options.limit !== undefined) {
    normalized.limit = options.limit;
  }

  if (keyColumns) {
    normalized.key_columns = keyColumns;
  }

  if (streamOptions) {
    normalized.subscription_options = streamOptions;
  }

  return Object.keys(normalized).length > 0 ? normalized : undefined;
}

export function normalizeLiveEventsOptions(
  options?: LiveEventsOptions,
): { batch_size?: number; last_rows?: number; from?: string; auto_fetch_batches?: boolean } | undefined {
  return normalizeLiveStreamOptions(options);
}

export function defaultRowKey(value: unknown): string | null {
  if (!value || typeof value !== 'object') {
    return null;
  }

  const candidate = (value as { id?: unknown }).id;
  if (typeof candidate === 'string') {
    return candidate;
  }
  if (candidate instanceof KalamCellValue) {
    return candidate.asString();
  }

  return null;
}

export function upsertLimited<T>(
  current: T[],
  incoming: T[],
  getKey: (row: T) => string | null | undefined,
  limit?: number,
): T[] {
  if (incoming.length === 0) {
    return typeof limit === 'number' && limit >= 0 && current.length > limit
      ? current.slice(-limit)
      : current;
  }

  const next = [...current];
  let keyedIndex: Map<string, number> | undefined;

  const ensureKeyedIndex = (): Map<string, number> => {
    if (!keyedIndex) {
      keyedIndex = new Map<string, number>();
      for (let index = 0; index < next.length; index += 1) {
        const key = getKey(next[index]);
        if (key) {
          keyedIndex.set(key, index);
        }
      }
    }
    return keyedIndex;
  };

  for (const item of incoming) {
    const key = getKey(item);
    if (key) {
      const indexByKey = ensureKeyedIndex();
      const existingIndex = indexByKey.get(key);
      if (existingIndex !== undefined) {
        next[existingIndex] = item;
        continue;
      }
      indexByKey.set(key, next.length);
    }

    next.push(item);
  }

  if (typeof limit === 'number' && limit >= 0 && next.length > limit) {
    return next.slice(-limit);
  }

  return next;
}

export function removeMaterializedRows<T>(
  current: T[],
  removed: T[],
  getKey: (row: T) => string | null | undefined,
): T[] {
  const keys = new Set<string>();
  for (const row of removed) {
    const key = getKey(row);
    if (typeof key === 'string' && key.length > 0) {
      keys.add(key);
    }
  }

  if (keys.size === 0) {
    return current;
  }

  return current.filter((row) => {
    const key = getKey(row);
    return !key || !keys.has(key);
  });
}

export function readSubscriptionInfos(
  raw: unknown,
  metadata: ReadonlyMap<string, LocalSubscriptionMetadata>,
): SubscriptionInfo[] {
  const parsed = parseWasmSubscriptions(raw);
  if (!parsed.parsed) {
    return localSubscriptionInfos(metadata);
  }

  return parsed.subscriptions.map((subscription) => {
    const local = metadata.get(subscription.id);
    return {
      id: subscription.id,
      tableName: subscription.query ?? local?.tableName ?? '',
      createdAt: new Date(local?.createdAtMs ?? 0),
      lastVersionId: parseVersionId(subscription.lastSeqId),
      closed: subscription.closed ?? false,
    };
  });
}

function parseWasmSubscriptions(raw: unknown): ParsedWasmSubscriptions {
  if (typeof raw !== 'string') {
    return {
      parsed: false,
      subscriptions: [],
    };
  }

  try {
    const parsed = JSON.parse(raw) as WasmSubscriptionSnapshot[];
    if (!Array.isArray(parsed)) {
      return {
        parsed: false,
        subscriptions: [],
      };
    }

    return {
      parsed: true,
      subscriptions: parsed,
    };
  } catch {
    return {
      parsed: false,
      subscriptions: [],
    };
  }
}

function localSubscriptionInfos(
  metadata: ReadonlyMap<string, LocalSubscriptionMetadata>,
): SubscriptionInfo[] {
  return Array.from(metadata.entries()).map(([id, local]) => ({
    id,
    tableName: local.tableName,
    createdAt: new Date(local.createdAtMs),
    lastVersionId: undefined,
    closed: false,
  }));
}

function parseVersionId(raw?: string): VersionId | undefined {
  if (!raw) {
    return undefined;
  }

  try {
    return VersionId.from(raw);
  } catch {
    return undefined;
  }
}

function wrapSubscriptionRows(rows: unknown): RowData[] | undefined {
  if (!Array.isArray(rows)) {
    return undefined;
  }

  return rows.map((row) => wrapRowMap((row ?? {}) as Record<string, unknown>));
}
