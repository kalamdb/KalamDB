import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  createRawSqlLiveDescriptor,
  type KalamDBClient,
  type LiveQueryController,
  type LiveQueryControllerSnapshot,
  type LiveQueryDescriptor,
  type RowData,
} from '@kalamdb/client';
import type { InferSelectModel, Table } from 'drizzle-orm';
import { useKalamClient } from '../context.js';
import type {
  DrizzleLiveQueryOptions,
  RawSqlLiveQueryOptions,
  SingleLiveQueryContext,
} from '../types.js';
import { useMutationActions } from './useMutationState.js';

const emptySnapshot: LiveQueryControllerSnapshot<never> = {
  rows: [],
  loading: true,
  connected: false,
  status: 'idle',
};

type OrmLiveCompiler = typeof import('@kalamdb/orm');

export function useLiveQuery<TTable extends Table>(
  options: DrizzleLiveQueryOptions<TTable>,
): SingleLiveQueryContext<InferSelectModel<TTable>>;
export function useLiveQuery<TTable extends Table, TSelected>(
  options: DrizzleLiveQueryOptions<TTable, TSelected>,
): TSelected;
export function useLiveQuery(
  options: RawSqlLiveQueryOptions,
): SingleLiveQueryContext<RowData>;
export function useLiveQuery<TSelected>(
  options: RawSqlLiveQueryOptions<TSelected>,
): TSelected;
export function useLiveQuery(options: DrizzleLiveQueryOptions<Table, unknown> | RawSqlLiveQueryOptions<unknown>) {
  const client = useKalamClient(options.client);
  const controllerRef = useRef<LiveQueryController<unknown> | null>(null);
  const [snapshot, setSnapshot] = useState<LiveQueryControllerSnapshot<unknown>>(emptySnapshot);
  const mutation = useMutationActions(client);

  useEffect(() => {
    let cancelled = false;

    async function start() {
      setSnapshot((current) => ({
        ...current,
        loading: true,
        connected: false,
        status: current.rows.length > 0 ? 'reconnecting' : 'loading',
        error: undefined,
      }));

      try {
        const descriptor = await resolveDescriptor(options);
        if (cancelled) {
          return;
        }

        const controller = createController(client, descriptor);
        controllerRef.current = controller;
        const unsubscribe = controller.subscribe((next) => {
          if (!cancelled) {
            setSnapshot(next);
          }
        });

        await controller.start();
        if (cancelled) {
          await unsubscribe();
          await controller.dispose();
        }
      } catch (error) {
        if (!cancelled) {
          setSnapshot({
            rows: [],
            loading: false,
            connected: false,
            status: 'error',
            error: toError(error),
          });
        }
      }
    }

    void start();

    return () => {
      cancelled = true;
      const controller = controllerRef.current;
      controllerRef.current = null;
      void controller?.dispose();
    };
  }, [
    client,
    options.limit,
    options.getKey,
    liveDepsKey(options.deps),
    isRawSqlOptions(options) ? options.query : options.table,
    isRawSqlOptions(options) ? undefined : options.where,
    isRawSqlOptions(options) ? undefined : options.orderBy,
  ]);

  const refetch = useCallback(async () => {
    await controllerRef.current?.refetch();
  }, []);

  const context = useMemo<SingleLiveQueryContext<unknown>>(() => ({
    rows: snapshot.rows,
    state: {
      ...snapshot,
      inserting: mutation.inserting,
      updating: mutation.updating,
      deleting: mutation.deleting,
      error: mutation.error ?? snapshot.error,
    },
    insert: mutation.insert,
    update: mutation.update,
    remove: mutation.remove,
    refetch,
  }), [mutation, refetch, snapshot]);

  return options.select ? options.select(context as never) : context;
}

function liveDepsKey(deps: readonly unknown[] | undefined): string {
  return deps?.map((value) => String(value)).join('\u001f') ?? '';
}

async function resolveDescriptor(
  options: DrizzleLiveQueryOptions<Table, unknown> | RawSqlLiveQueryOptions<unknown>,
): Promise<LiveQueryDescriptor<unknown>> {
  if (isRawSqlOptions(options)) {
    if ('table' in options) {
      throw new Error('LiveQuery accepts either query or table, not both.');
    }

    return createRawSqlLiveDescriptor(options.query, {
      limit: options.limit,
      getKey: options.getKey,
    }) as LiveQueryDescriptor<unknown>;
  }

  if ('query' in options) {
    throw new Error('LiveQuery accepts either query or table, not both.');
  }

  const orm = await loadOrm();
  return orm.compileLiveTableDescriptor(options.table, {
    where: options.where?.(options.table),
    orderBy: options.orderBy?.(options.table),
    limit: options.limit,
    getKey: options.getKey,
  }) as LiveQueryDescriptor<unknown>;
}

function createController(
  client: KalamDBClient,
  descriptor: LiveQueryDescriptor<unknown>,
): LiveQueryController<unknown> {
  if ('createLiveQueryController' in client && typeof client.createLiveQueryController === 'function') {
    return client.createLiveQueryController(descriptor) as LiveQueryController<unknown>;
  }

  throw new Error('@kalamdb/client is missing createLiveQueryController(). Rebuild or update @kalamdb/client.');
}

async function loadOrm(): Promise<OrmLiveCompiler> {
  try {
    return await import('@kalamdb/orm');
  } catch (error) {
    throw new Error(`Typed live queries require @kalamdb/orm and drizzle-orm to be installed. ${String(error)}`);
  }
}

function isRawSqlOptions(
  options: DrizzleLiveQueryOptions<Table, unknown> | RawSqlLiveQueryOptions<unknown>,
): options is RawSqlLiveQueryOptions<unknown> {
  return 'query' in options && typeof options.query === 'string';
}

function toError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}