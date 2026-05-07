import { useCallback, useMemo, useReducer, type Dispatch } from 'react';
import type { KalamDBClient } from '@kalamdb/client';
import { getTableColumns, type Table } from 'drizzle-orm';
import type { InsertAction, MutationActions, MutationState, RemoveAction, UpdateAction } from '../types.js';

export type MutationAction =
  | { type: 'insert:start' }
  | { type: 'insert:finish' }
  | { type: 'update:start'; rowKey: string }
  | { type: 'update:finish'; rowKey: string }
  | { type: 'delete:start'; rowKey: string }
  | { type: 'delete:finish'; rowKey: string }
  | { type: 'error'; error: Error };

const initialMutationState: MutationState = {
  inserting: false,
  updating: new Set<string>(),
  deleting: new Set<string>(),
};

const kalamTableConfigSymbol = Symbol.for('kalamdb.orm.tableConfig');
const drizzleNameSymbol = Symbol.for('drizzle:Name');
const drizzleBaseNameSymbol = Symbol.for('drizzle:BaseName');

export function useMutationState(): [MutationState, Dispatch<MutationAction>] {
  return useReducer(mutationReducer, initialMutationState);
}

export function useMutationActions(client: KalamDBClient): MutationState & MutationActions {
  const [state, dispatch] = useMutationState();

  const runInsert = useCallback(async (target: string | Table, row: Record<string, unknown>) => {
    dispatch({ type: 'insert:start' });
    try {
      await client.insert(resolveMutationTableName(target), normalizeMutationPayload(target, row));
      dispatch({ type: 'insert:finish' });
    } catch (error) {
      dispatch({ type: 'error', error: toError(error) });
      throw error;
    }
  }, [client]);

  const runUpdate = useCallback(async (target: string | Table, rowKey: string, patch: Record<string, unknown>) => {
    dispatch({ type: 'update:start', rowKey });
    try {
      await client.update(resolveMutationTableName(target), rowKey, normalizeMutationPayload(target, patch));
      dispatch({ type: 'update:finish', rowKey });
    } catch (error) {
      dispatch({ type: 'error', error: toError(error) });
      throw error;
    }
  }, [client]);

  const runDelete = useCallback(async (target: string | Table, rowKey: string) => {
    dispatch({ type: 'delete:start', rowKey });
    try {
      await client.delete(resolveMutationTableName(target), rowKey);
      dispatch({ type: 'delete:finish', rowKey });
    } catch (error) {
      dispatch({ type: 'error', error: toError(error) });
      throw error;
    }
  }, [client]);

  const insert = useMemo(() => ((target: string | Table, row?: Record<string, unknown>) => {
    if (row !== undefined) {
      return runInsert(target, row);
    }

    return {
      values: (value: Record<string, unknown>) => runInsert(target, value),
    };
  }) as InsertAction, [runInsert]);

  const update = useMemo(() => ((target: string | Table, rowKey: string, patch?: Record<string, unknown>) => {
    if (patch !== undefined) {
      return runUpdate(target, rowKey, patch);
    }

    return {
      set: (value: Record<string, unknown>) => runUpdate(target, rowKey, value),
    };
  }) as UpdateAction, [runUpdate]);

  const remove = useMemo(() => ((target: string | Table, rowKey: string) => runDelete(target, rowKey)) as RemoveAction, [runDelete]);

  return useMemo(() => ({
    ...state,
    insert,
    update,
    remove,
  }), [insert, remove, state, update]);
}

export function resolveMutationTableName(target: string | Table): string {
  if (typeof target === 'string') {
    return target;
  }

  const tableObject = target as unknown as Record<PropertyKey, unknown>;
  const kalamConfig = tableObject[kalamTableConfigSymbol] as { qualifiedName?: string } | undefined;
  if (kalamConfig?.qualifiedName) {
    return kalamConfig.qualifiedName;
  }

  const drizzleName = tableObject[drizzleNameSymbol] ?? tableObject[drizzleBaseNameSymbol];
  if (typeof drizzleName === 'string' && drizzleName.length > 0) {
    return drizzleName;
  }

  throw new Error('Unable to resolve KalamDB table name from the provided mutation target.');
}

function normalizeMutationPayload(target: string | Table, payload: Record<string, unknown>): Record<string, unknown> {
  if (typeof target === 'string') {
    return payload;
  }

  const columns = getTableColumns(target) as Record<string, { name?: string }>;
  return Object.fromEntries(
    Object.entries(payload).map(([key, value]) => {
      const columnName = columns[key]?.name;
      return [columnName ?? key, value];
    }),
  );
}

function mutationReducer(state: MutationState, action: MutationAction): MutationState {
  switch (action.type) {
    case 'insert:start':
      return { ...state, inserting: true, error: undefined };
    case 'insert:finish':
      return { ...state, inserting: false };
    case 'update:start':
      return { ...state, updating: addKey(state.updating, action.rowKey), error: undefined };
    case 'update:finish':
      return { ...state, updating: removeKey(state.updating, action.rowKey) };
    case 'delete:start':
      return { ...state, deleting: addKey(state.deleting, action.rowKey), error: undefined };
    case 'delete:finish':
      return { ...state, deleting: removeKey(state.deleting, action.rowKey) };
    case 'error':
      return {
        inserting: false,
        updating: new Set<string>(),
        deleting: new Set<string>(),
        error: action.error,
      };
  }
}

function addKey(values: Set<string>, rowKey: string): Set<string> {
  const next = new Set(values);
  next.add(rowKey);
  return next;
}

function removeKey(values: Set<string>, rowKey: string): Set<string> {
  const next = new Set(values);
  next.delete(rowKey);
  return next;
}

function toError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}