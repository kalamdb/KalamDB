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
import { liveQuerySignature } from '../internal/signature.js';
import { useMutationActions } from './useMutationState.js';

const initialSnapshot: LiveQueryControllerSnapshot<unknown> = {
  rows: [],
  loading: true,
  connected: false,
  status: 'loading',
};

type OrmModule = typeof import('@kalamdb/orm');

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
export function useLiveQuery(
  options: DrizzleLiveQueryOptions<Table, unknown> | RawSqlLiveQueryOptions<unknown>,
) {
  const client = useKalamClient(options.client);
  const controllerRef = useRef<LiveQueryController<unknown> | null>(null);
  const [snapshot, setSnapshot] = useState<LiveQueryControllerSnapshot<unknown>>(initialSnapshot);
  const mutation = useMutationActions(client);
  const signature = liveQuerySignature(options);

  useEffect(() => {
    let cancelled = false;
    let unsubscribe: (() => Promise<void>) | null = null;

    setSnapshot((current) => ({
      ...current,
      loading: true,
      connected: false,
      status: current.rows.length > 0 ? 'reconnecting' : 'loading',
      error: undefined,
    }));

    async function start() {
      try {
        const descriptor = await resolveDescriptor(options);
        if (cancelled) return;

        const controller = createController(client, descriptor);
        controllerRef.current = controller;
        unsubscribe = controller.subscribe((next) => {
          if (!cancelled) setSnapshot(next);
        });

        await controller.start();
        if (cancelled) {
          const detach = unsubscribe;
          unsubscribe = null;
          await detach?.();
          await controller.dispose();
        }
      } catch (error) {
        if (cancelled) return;
        setSnapshot({
          rows: [],
          loading: false,
          connected: false,
          status: 'error',
          error: toError(error),
        });
      }
    }

    void start();

    return () => {
      cancelled = true;
      const controller = controllerRef.current;
      controllerRef.current = null;
      const detach = unsubscribe;
      unsubscribe = null;
      void (async () => {
        try { await detach?.(); } catch { /* swallow: teardown */ }
        try { await controller?.dispose(); } catch { /* swallow: teardown */ }
      })();
    };
  }, [client, signature]);

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
    clearError: mutation.clearError,
    refetch,
  }), [mutation, refetch, snapshot]);

  return options.select ? options.select(context as never) : context;
}

async function resolveDescriptor(
  options: DrizzleLiveQueryOptions<Table, unknown> | RawSqlLiveQueryOptions<unknown>,
): Promise<LiveQueryDescriptor<unknown>> {
  const hasQuery = 'query' in options && typeof options.query === 'string';
  const hasTable = 'table' in options;
  if (hasQuery && hasTable) {
    throw new Error('LiveQuery accepts either query or table, not both.');
  }

  if (hasQuery) {
    const sql = options as RawSqlLiveQueryOptions<unknown>;
    return createRawSqlLiveDescriptor(sql.query, {
      limit: sql.limit,
      getKey: sql.getKey,
    }) as LiveQueryDescriptor<unknown>;
  }

  const drizzle = options as DrizzleLiveQueryOptions<Table, unknown>;
  const orm = await loadOrm();
  return orm.compileLiveTableDescriptor(drizzle.table, {
    where: drizzle.where?.(drizzle.table),
    orderBy: drizzle.orderBy?.(drizzle.table),
    limit: drizzle.limit,
    getKey: drizzle.getKey,
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

async function loadOrm(): Promise<OrmModule> {
  try {
    return await import('@kalamdb/orm');
  } catch (error) {
    throw new Error(`Typed live queries require @kalamdb/orm and drizzle-orm to be installed. ${String(error)}`);
  }
}

function toError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}
