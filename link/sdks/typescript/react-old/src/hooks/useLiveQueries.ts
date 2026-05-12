import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type {
  KalamDBClient,
  LiveQueryController,
  LiveQueryControllerSnapshot,
  LiveQueryDescriptor,
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
import { liveQueriesSignature } from '../internal/signature.js';
import { useMutationActions } from './useMutationState.js';

type OrmModule = typeof import('@kalamdb/orm');
type SnapshotMap = Record<string, LiveQueryControllerSnapshot<unknown>>;

const initialSnapshot: LiveQueryControllerSnapshot<unknown> = {
  rows: [],
  loading: true,
  connected: false,
  status: 'loading',
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
    // Install new controller map BEFORE awaiting anything so a racing cleanup
    // (StrictMode rapid mount/unmount) tears down the right generation.
    const previousControllers = controllersRef.current;
    const controllers = new Map<string, LiveQueryController<unknown>>();
    controllersRef.current = controllers;

    setSnapshots((current) => initialSnapshots(options.queries, current));

    async function start() {
      try { await disposeControllers(previousControllers); } catch { /* swallow */ }

      let orm: OrmModule;
      try {
        orm = await loadOrm();
      } catch (error) {
        if (cancelled) return;
        markAllFailed(setSnapshots, entries, toError(error));
        return;
      }

      // Each query gets its own try/catch so one failure does not erase the
      // snapshots of its siblings.
      await Promise.all(entries.map(async ([name, definition]) => {
        try {
          const descriptor = compileDescriptor(orm, definition);
          if (cancelled) return;

          const controller = createController(client, descriptor);
          if (cancelled) { await controller.dispose(); return; }
          controllers.set(name, controller);

          const unsubscribe = controller.subscribe((snapshot) => {
            if (cancelled) return;
            setSnapshots((current) => ({ ...current, [name]: snapshot }));
          });

          try {
            await controller.start();
          } catch (startError) {
            try { await unsubscribe(); } catch { /* swallow */ }
            throw startError;
          }
        } catch (perQueryError) {
          if (cancelled) return;
          markOneFailed(setSnapshots, name, toError(perQueryError));
        }
      }));
    }

    void start();

    return () => {
      cancelled = true;
      const toDispose = controllersRef.current;
      controllersRef.current = new Map();
      void disposeControllers(toDispose);
    };
  }, [client, querySignature]);

  const refetch = useCallback(async () => {
    await Promise.all([...controllersRef.current.values()].map((c) => c.refetch()));
  }, []);

  const context = useMemo(() => {
    const entries = Object.entries(options.queries);
    const queryContexts = Object.fromEntries(entries.map(([name]) => [
      name,
      buildSingleContext(snapshots[name] ?? initialSnapshot, mutation, refetch),
    ]));

    const querySnapshots = entries.map(([name]) => snapshots[name] ?? initialSnapshot);
    const aggregateError = mutation.error ?? querySnapshots.find((s) => s.error)?.error;

    return {
      ...queryContexts,
      state: {
        loading: querySnapshots.length > 0 && querySnapshots.some((s) => s.loading),
        connected: querySnapshots.length > 0 && querySnapshots.every((s) => s.connected),
        inserting: mutation.inserting,
        updating: mutation.updating,
        deleting: mutation.deleting,
        ...(aggregateError ? { error: aggregateError } : {}),
      },
      insert: mutation.insert,
      update: mutation.update,
      remove: mutation.remove,
      clearError: mutation.clearError,
      refetch,
    } as MultiLiveQueryContext<TQueries>;
  }, [mutation, querySignature, refetch, snapshots]);

  return options.select ? options.select(context) : context;
}

function buildSingleContext(
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
    clearError: mutation.clearError,
    refetch,
  };
}

function initialSnapshots(queries: LiveQueriesDefinition, current: SnapshotMap = {}): SnapshotMap {
  return Object.fromEntries(Object.keys(queries).map((name) => [
    name,
    {
      ...(current[name] ?? initialSnapshot),
      loading: true,
      connected: false,
      status: current[name]?.rows.length ? 'reconnecting' : 'loading',
      error: undefined,
    } satisfies LiveQueryControllerSnapshot<unknown>,
  ]));
}

function markOneFailed(
  setSnapshots: React.Dispatch<React.SetStateAction<SnapshotMap>>,
  name: string,
  error: Error,
): void {
  setSnapshots((current) => ({
    ...current,
    [name]: {
      ...(current[name] ?? initialSnapshot),
      loading: false,
      connected: false,
      status: 'error',
      error,
    } satisfies LiveQueryControllerSnapshot<unknown>,
  }));
}

function markAllFailed(
  setSnapshots: React.Dispatch<React.SetStateAction<SnapshotMap>>,
  entries: Array<[string, unknown]>,
  error: Error,
): void {
  setSnapshots((current) => Object.fromEntries(
    entries.map(([name]) => [
      name,
      {
        ...(current[name] ?? initialSnapshot),
        loading: false,
        connected: false,
        status: 'error',
        error,
      } satisfies LiveQueryControllerSnapshot<unknown>,
    ]),
  ));
}

async function disposeControllers(controllers: Map<string, LiveQueryController<unknown>>): Promise<void> {
  await Promise.all([...controllers.values()].map((c) => c.dispose().catch(() => undefined)));
  controllers.clear();
}

function compileDescriptor(
  orm: OrmModule,
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
