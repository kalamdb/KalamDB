import { useCallback, useMemo, useReducer, type Dispatch } from 'react';
import type { KalamDBClient } from '@kalamdb/client';
import { getTableColumns, type Table } from 'drizzle-orm';
import type {
  InsertAction,
  MutationActions,
  MutationState,
  RemoveAction,
  RowKey,
  UpdateAction,
} from '../types.js';

export type MutationAction =
  | { type: 'insert:start' }
  | { type: 'insert:finish' }
  | { type: 'update:start'; rowKey: RowKey }
  | { type: 'update:finish'; rowKey: RowKey }
  | { type: 'delete:start'; rowKey: RowKey }
  | { type: 'delete:finish'; rowKey: RowKey }
  | { type: 'error'; error: Error }
  | { type: 'clear-error' };

interface ReducerState {
  insertingCount: number;
  updating: Set<RowKey>;
  deleting: Set<RowKey>;
  error?: Error;
}

const KALAM_TABLE_CONFIG = Symbol.for('kalamdb.orm.tableConfig');
const DRIZZLE_NAME = Symbol.for('drizzle:Name');
const DRIZZLE_BASE_NAME = Symbol.for('drizzle:BaseName');

const INITIAL: ReducerState = {
  insertingCount: 0,
  updating: new Set<RowKey>(),
  deleting: new Set<RowKey>(),
};

function toPublic(state: ReducerState): MutationState {
  return {
    inserting: state.insertingCount > 0,
    updating: state.updating,
    deleting: state.deleting,
    error: state.error,
  };
}

export function useMutationState(): [MutationState, Dispatch<MutationAction>] {
  const [state, dispatch] = useReducer(reducer, INITIAL);
  const publicState = useMemo(() => toPublic(state), [state]);
  return [publicState, dispatch];
}

export function useMutationActions(client: KalamDBClient): MutationState & MutationActions {
  const [state, dispatch] = useReducer(reducer, INITIAL);

  const runInsert = useCallback(async (target: string | Table, row: Record<string, unknown>) => {
    dispatch({ type: 'insert:start' });
    try {
      await client.insert(resolveMutationTableName(target), normalizeMutationPayload(target, row));
    } catch (error) {
      dispatch({ type: 'error', error: toError(error) });
      throw error;
    } finally {
      dispatch({ type: 'insert:finish' });
    }
  }, [client]);

  const runUpdate = useCallback(async (target: string | Table, rowKey: RowKey, patch: Record<string, unknown>) => {
    dispatch({ type: 'update:start', rowKey });
    try {
      await client.update(resolveMutationTableName(target), rowKey, normalizeMutationPayload(target, patch));
    } catch (error) {
      dispatch({ type: 'error', error: toError(error) });
      throw error;
    } finally {
      dispatch({ type: 'update:finish', rowKey });
    }
  }, [client]);

  const runDelete = useCallback(async (target: string | Table, rowKey: RowKey) => {
    dispatch({ type: 'delete:start', rowKey });
    try {
      await client.delete(resolveMutationTableName(target), rowKey);
    } catch (error) {
      dispatch({ type: 'error', error: toError(error) });
      throw error;
    } finally {
      dispatch({ type: 'delete:finish', rowKey });
    }
  }, [client]);

  const insert = useMemo(() => ((target: string | Table, row?: Record<string, unknown>) => {
    if (row !== undefined) return runInsert(target, row);
    return { values: (value: Record<string, unknown>) => runInsert(target, value) };
  }) as InsertAction, [runInsert]);

  const update = useMemo(() => ((target: string | Table, rowKey: RowKey, patch?: Record<string, unknown>) => {
    if (patch !== undefined) return runUpdate(target, rowKey, patch);
    return { set: (value: Record<string, unknown>) => runUpdate(target, rowKey, value) };
  }) as UpdateAction, [runUpdate]);

  const remove = useMemo(
    () => ((target: string | Table, rowKey: RowKey) => runDelete(target, rowKey)) as RemoveAction,
    [runDelete],
  );

  const clearError = useCallback(() => dispatch({ type: 'clear-error' }), []);

  const publicState = useMemo(() => toPublic(state), [state]);

  return useMemo(() => ({
    ...publicState,
    insert,
    update,
    remove,
    clearError,
  }), [clearError, insert, publicState, remove, update]);
}

/**
 * Resolve the wire-format KalamDB table name from a mutation target.
 * Prefers `@kalamdb/orm`'s `qualifiedName` (set by `kTable.user(...)`) so
 * `schema.table` is preserved; falls back to Drizzle's name symbols for raw
 * `pgTable` usage.
 */
export function resolveMutationTableName(target: string | Table): string {
  if (typeof target === 'string') return target;

  const obj = target as unknown as Record<PropertyKey, unknown>;
  const kalam = obj[KALAM_TABLE_CONFIG] as { qualifiedName?: string } | undefined;
  if (kalam?.qualifiedName) return kalam.qualifiedName;

  const drizzle = obj[DRIZZLE_NAME] ?? obj[DRIZZLE_BASE_NAME];
  if (typeof drizzle === 'string' && drizzle.length > 0) return drizzle;

  throw new Error('Unable to resolve KalamDB table name from the provided mutation target.');
}

/**
 * Translate a Drizzle-shaped payload to the wire format:
 * - Maps camelCase JS keys → snake_case DB column names via `getTableColumns`.
 * - Applies each column's `mapToDriverValue` encoder (e.g. custom types,
 *   JSON serializers) for non-null values. Mirrors Drizzle's own encoder
 *   pipeline so user-defined `customType` round-trips correctly.
 */
function normalizeMutationPayload(
  target: string | Table,
  payload: Record<string, unknown>,
): Record<string, unknown> {
  if (typeof target === 'string') return payload;

  const columns = getTableColumns(target) as Record<
    string,
    { name?: string; mapToDriverValue?: (value: unknown) => unknown }
  >;
  return Object.fromEntries(
    Object.entries(payload).map(([key, value]) => {
      const column = columns[key];
      const columnName = column?.name ?? key;
      const encoded = value != null && typeof column?.mapToDriverValue === 'function'
        ? column.mapToDriverValue(value)
        : value;
      return [columnName, encoded];
    }),
  );
}

function reducer(state: ReducerState, action: MutationAction): ReducerState {
  switch (action.type) {
    case 'insert:start':
      return { ...state, insertingCount: state.insertingCount + 1, error: undefined };
    case 'insert:finish':
      return { ...state, insertingCount: Math.max(0, state.insertingCount - 1) };
    case 'update:start':
      return { ...state, updating: withKey(state.updating, action.rowKey), error: undefined };
    case 'update:finish':
      return { ...state, updating: withoutKey(state.updating, action.rowKey) };
    case 'delete:start':
      return { ...state, deleting: withKey(state.deleting, action.rowKey), error: undefined };
    case 'delete:finish':
      return { ...state, deleting: withoutKey(state.deleting, action.rowKey) };
    case 'error':
      return { ...state, error: action.error };
    case 'clear-error':
      return state.error ? { ...state, error: undefined } : state;
  }
}

function withKey(values: Set<RowKey>, key: RowKey): Set<RowKey> {
  const next = new Set(values);
  next.add(key);
  return next;
}

function withoutKey(values: Set<RowKey>, key: RowKey): Set<RowKey> {
  const next = new Set(values);
  next.delete(key);
  return next;
}

function toError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}
