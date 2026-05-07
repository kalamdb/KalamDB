import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type {
  KalamDBClient,
  LiveQueryController,
  LiveQueryControllerSnapshot,
  LiveQueryDescriptor,
  RowData,
} from '@kalamdb/client';
import type { Table } from 'drizzle-orm';
import { useKalamClient } from '../context.js';
import type {
  DrizzleLiveQueryDefinition,
  LiveQueriesDefinition,
  MultiLiveQueryContext,
  SingleLiveQueryContext,
  UseLiveQueriesOptions,
} from '../types.js';
import { useMutationActions } from './useMutationState.js';

type OrmLiveCompiler = typeof import('@kalamdb/orm');
type SnapshotMap = Record<string, LiveQueryControllerSnapshot<unknown>>;

const idleSnapshot: LiveQueryControllerSnapshot<unknown> = {
  rows: [],
  loading: true,
  connected: false,
  status: 'idle',
};

export function useLiveQueries<TQueries extends LiveQueriesDefinition>(
  options: UseLiveQueriesOptions<TQueries>,
): MultiLiveQueryContext<TQueries>;
export function useLiveQueries<TQueries extends LiveQueriesDefinition, TSelected>(
  options: UseLiveQueriesOptions<TQueries, TSelected>,
): TSelected;
export function useLiveQueries<TQueries extends LiveQueriesDefinition, TSelected>(
  options: UseLiveQueriesOptions<TQueries, TSelected>,
) {
  const client = useKalamClient(options.client);
  const mutation = useMutationActions(client);
  const controllersRef = useRef<Map<string, LiveQueryController<unknown>>>(new Map());
  const [snapshots, setSnapshots] = useState<SnapshotMap>(() => initialSnapshots(options.queries));
  const querySignature = liveQueriesSignature(options.queries, options.deps);

  useEffect(() => {
    let cancelled = false;
    const entries = Object.entries(options.queries);

    setSnapshots((current) => initialSnapshots(options.queries, current));

    async function start() {
      await disposeControllers(controllersRef.current);
      const controllers = new Map<string, LiveQueryController<unknown>>();
      controllersRef.current = controllers;

      try {
        const orm = await loadOrm();

        await Promise.all(entries.map(async ([name, definition]) => {
          const descriptor = compileDescriptor(orm, definition);
          if (cancelled) {
            return;
          }

          const controller = createController(client, descriptor);
          controllers.set(name, controller);
          controller.subscribe((snapshot) => {
            if (!cancelled) {
              setSnapshots((current) => ({ ...current, [name]: snapshot }));
            }
          });
          await controller.start();
        }));
      } catch (error) {
        if (!cancelled) {
          const failed = toError(error);
          setSnapshots((current) => Object.fromEntries(
            entries.map(([name]) => [
              name,
              {
                ...(current[name] ?? idleSnapshot),
                loading: false,
                connected: false,
                status: 'error',
                error: failed,
              } satisfies LiveQueryControllerSnapshot<unknown>,
            ]),
          ));
        }
      }
    }

    void start();

    return () => {
      cancelled = true;
      const controllers = controllersRef.current;
      controllersRef.current = new Map();
      void disposeControllers(controllers);
    };
  }, [client, querySignature]);

  const refetch = useCallback(async () => {
    await Promise.all([...controllersRef.current.values()].map((controller) => controller.refetch()));
  }, []);

  const context = useMemo(() => {
    const entries = Object.entries(options.queries);
    const queryContexts = Object.fromEntries(entries.map(([name]) => [
      name,
      singleContext(snapshots[name] ?? idleSnapshot, mutation, refetch),
    ]));

    const querySnapshots = entries.map(([name]) => snapshots[name] ?? idleSnapshot);
    const aggregateError = mutation.error ?? querySnapshots.find((snapshot) => snapshot.error)?.error;

    return {
      ...queryContexts,
      state: {
        loading: querySnapshots.length > 0 && querySnapshots.some((snapshot) => snapshot.loading),
        connected: querySnapshots.length > 0 && querySnapshots.every((snapshot) => snapshot.connected),
        inserting: mutation.inserting,
        updating: mutation.updating,
        deleting: mutation.deleting,
        ...(aggregateError ? { error: aggregateError } : {}),
      },
      insert: mutation.insert,
      update: mutation.update,
      remove: mutation.remove,
      refetch,
    } as MultiLiveQueryContext<TQueries>;
  }, [mutation, options.queries, refetch, snapshots]);

  return options.select ? options.select(context) : context;
}

function singleContext(
  snapshot: LiveQueryControllerSnapshot<unknown>,
  mutation: ReturnType<typeof useMutationActions>,
  refetch: () => Promise<void>,
): SingleLiveQueryContext<unknown> {
  return {
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
  };
}

function initialSnapshots(queries: LiveQueriesDefinition, current: SnapshotMap = {}): SnapshotMap {
  return Object.fromEntries(Object.keys(queries).map((name) => [
    name,
    {
      ...(current[name] ?? idleSnapshot),
      loading: true,
      connected: false,
      status: current[name]?.rows.length ? 'reconnecting' : 'loading',
      error: undefined,
    } satisfies LiveQueryControllerSnapshot<unknown>,
  ]));
}

async function disposeControllers(controllers: Map<string, LiveQueryController<unknown>>): Promise<void> {
  await Promise.all([...controllers.values()].map((controller) => controller.dispose()));
  controllers.clear();
}

function compileDescriptor(
  orm: OrmLiveCompiler,
  definition: DrizzleLiveQueryDefinition<Table>,
): LiveQueryDescriptor<unknown> {
  return orm.compileLiveTableDescriptor(definition.table, {
    where: definition.where?.(definition.table),
    orderBy: definition.orderBy?.(definition.table),
    limit: definition.limit,
    getKey: definition.getKey,
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

function liveQueriesSignature(queries: LiveQueriesDefinition, deps: readonly unknown[] | undefined): string {
  return Object.entries(queries).map(([name, definition]) => [
    name,
    tableName(definition.table),
    definition.limit ?? '',
    getKeyName(definition.getKey),
    liveDepsKey(definition.deps),
  ].join(':')).concat(liveDepsKey(deps)).join('|');
}

function tableName(table: Table): string {
  const tableObject = table as unknown as Record<PropertyKey, unknown>;
  const kalamConfig = tableObject[Symbol.for('kalamdb.orm.tableConfig')] as { qualifiedName?: string } | undefined;
  const drizzleName = tableObject[Symbol.for('drizzle:Name')] ?? tableObject[Symbol.for('drizzle:BaseName')];
  return kalamConfig?.qualifiedName ?? (typeof drizzleName === 'string' ? drizzleName : String(table));
}

function getKeyName(getKey: unknown): string {
  return Array.isArray(getKey) ? getKey.join(',') : String(getKey ?? '');
}

function liveDepsKey(deps: readonly unknown[] | undefined): string {
  return deps?.map((value) => String(value)).join('\u001f') ?? '';
}

function toError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}