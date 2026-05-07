# Contract: `@kalamdb/react` Public API

## Package Scope

The package exposes React-facing primitives for KalamDB live data. Shared subscription orchestration remains in `@kalamdb/client`; this contract only defines the React package surface and the observable behavior it depends on. For complex screens, hooks are the primary composition surface and components are thin wrappers over those hooks.

## Public Exports

```ts
import type { ReactNode } from 'react';
import type { KalamDBClient, RowData } from '@kalamdb/client';
import type { InferInsertModel, InferSelectModel, SQLWrapper, Table } from 'drizzle-orm';

export interface KalamProviderProps {
  client: KalamDBClient;
  children: ReactNode;
}

export function KalamProvider(props: KalamProviderProps): ReactNode;

export interface LiveQueryState<TRow> {
  loading: boolean;
  connected: boolean;
  inserting: boolean;
  updating: Set<string>;
  deleting: Set<string>;
  error?: Error;
}

export interface SingleLiveQueryContext<TRow> {
  rows: TRow[];
  state: LiveQueryState<TRow>;
  insert: InsertAction;
  update: UpdateAction;
  remove: RemoveAction;
  refetch: () => Promise<void>;
}

export interface MultiLiveQueryAggregateState {
  loading: boolean;
  connected: boolean;
  error?: Error;
}

export type InsertAction = {
  <TTable extends Table>(table: TTable): {
    values: (row: InferInsertModel<TTable>) => Promise<void>;
  };
  (tableName: string, row: Record<string, unknown>): Promise<void>;
};

export type UpdateAction = {
  <TTable extends Table>(table: TTable, rowKey: string): {
    set: (patch: Partial<InferInsertModel<TTable>>) => Promise<void>;
  };
  (tableName: string, rowKey: string, patch: Record<string, unknown>): Promise<void>;
};

export type RemoveAction = {
  <TTable extends Table>(table: TTable, rowKey: string): Promise<void>;
  (tableName: string, rowKey: string): Promise<void>;
};

export type DrizzleLiveQueryProps<TTable extends Table> = {
  client?: KalamDBClient;
  table: TTable;
  where?: (table: TTable) => SQLWrapper;
  orderBy?: (table: TTable) => SQLWrapper | SQLWrapper[];
  limit?: number;
  children: (ctx: SingleLiveQueryContext<InferSelectModel<TTable>>) => ReactNode;
};

export type SqlLiveQueryProps = {
  client?: KalamDBClient;
  query: string;
  limit?: number;
  getKey?: string | string[] | ((row: RowData) => string | null | undefined);
  children: (ctx: SingleLiveQueryContext<RowData>) => ReactNode;
};

export type LiveQueryProps<TTable extends Table> = DrizzleLiveQueryProps<TTable> | SqlLiveQueryProps;

export type UseDrizzleLiveQueryOptions<TTable extends Table, TSelected = SingleLiveQueryContext<InferSelectModel<TTable>>> = Omit<DrizzleLiveQueryProps<TTable>, 'children'> & {
  select?: (ctx: SingleLiveQueryContext<InferSelectModel<TTable>>) => TSelected;
};

export type UseSqlLiveQueryOptions<TSelected = SingleLiveQueryContext<RowData>> = Omit<SqlLiveQueryProps, 'children'> & {
  select?: (ctx: SingleLiveQueryContext<RowData>) => TSelected;
};

export function useLiveQuery<TTable extends Table>(options: UseDrizzleLiveQueryOptions<TTable>): SingleLiveQueryContext<InferSelectModel<TTable>>;
export function useLiveQuery<TTable extends Table, TSelected>(options: UseDrizzleLiveQueryOptions<TTable, TSelected>): TSelected;
export function useLiveQuery(options: UseSqlLiveQueryOptions): SingleLiveQueryContext<RowData>;
export function useLiveQuery<TSelected>(options: UseSqlLiveQueryOptions<TSelected>): TSelected;

export function LiveQuery<TTable extends Table>(props: LiveQueryProps<TTable>): ReactNode;

export type LiveQueriesDefinition = Record<string, {
  table: Table;
  where?: (table: Table) => SQLWrapper;
  orderBy?: (table: Table) => SQLWrapper | SQLWrapper[];
  limit?: number;
}>;

export type InferLiveQueryRow<TDefinition> =
  TDefinition extends { table: infer TTable extends Table }
    ? InferSelectModel<TTable>
    : RowData;

export type MultiLiveQueryContext<TQueries extends LiveQueriesDefinition> = {
  [K in keyof TQueries]: SingleLiveQueryContext<InferLiveQueryRow<TQueries[K]>>;
} & {
  state: MultiLiveQueryAggregateState;
  insert: InsertAction;
  update: UpdateAction;
  remove: RemoveAction;
};

export interface UseLiveQueriesOptions<TQueries extends LiveQueriesDefinition, TSelected = MultiLiveQueryContext<TQueries>> {
  client?: KalamDBClient;
  queries: TQueries;
  select?: (ctx: MultiLiveQueryContext<TQueries>) => TSelected;
}

export function useLiveQueries<TQueries extends LiveQueriesDefinition>(options: UseLiveQueriesOptions<TQueries>): MultiLiveQueryContext<TQueries>;
export function useLiveQueries<TQueries extends LiveQueriesDefinition, TSelected>(options: UseLiveQueriesOptions<TQueries, TSelected>): TSelected;

export interface LiveQueriesProps<TQueries extends LiveQueriesDefinition> {
  client?: KalamDBClient;
  queries: TQueries;
  children: (ctx: MultiLiveQueryContext<TQueries>) => ReactNode;
}

export function LiveQueries<TQueries extends LiveQueriesDefinition>(props: LiveQueriesProps<TQueries>): ReactNode;
```

## Behavioral Invariants

- `useLiveQuery` and `useLiveQueries` are the primary composition APIs for advanced screens; `LiveQuery` and `LiveQueries` wrap the same underlying behavior for declarative rendering.
- `LiveQuery` accepts exactly one mode: Drizzle or raw SQL.
- `LiveQueries` initial release accepts typed table-based definitions only.
- When `client` prop is omitted, `LiveQuery` and `LiveQueries` resolve the client from `KalamProvider`.
- Query-definition validation failures must surface before any subscription starts.
- `rows` always reflects the last confirmed materialized result and must remain available during transient disconnects.
- `state.loading` remains `true` until the first stable result is ready.
- `state.connected` reflects live-connection health, not whether initial rows exist.
- `state.inserting`, `state.updating`, and `state.deleting` reflect in-flight mutation work but do not imply optimistic row mutation.
- `refetch()` re-runs the initial fetch and rebuilds the live session without requiring a component remount.
- `select` must be a pure projection over current live state and must not become a second authoritative data store.
- Complex screens such as assistant UIs should be able to derive pending approvals, active tool calls, typing users, or online participants directly from `useLiveQueries` output rather than via effect-managed local copies.

## Raw SQL Compatibility Contract

- Raw SQL mode is valid only for the live-compatible query subset supported by the shared client controller in v1.
- Ordered or limited SQL may be accepted only when the controller can normalize it into a live-safe subscription plus client-side projection.
- Unsupported SQL shapes must fail with a descriptive runtime error instead of silently degrading to polling or stale behavior.

## Admin UI Integration Contract

- The package must accept an existing `KalamDBClient` instance created by `ui/src/lib/kalam-client.ts`.
- The package must not create a second implicit authentication flow.
- Multiple live queries in one screen must reuse the same provided client instance.

## Assistant Workflow Coverage Contract

- The public API must support one screen combining datasets such as `messages`, `toolCalls`, `toolResults`, `typing`, `presence`, and `approvals` through a single `useLiveQueries` or `LiveQueries` entry point.
- Approval actions, tool-state updates, and chat mutations must remain target-aware so one dataset can mutate without forcing unrelated datasets to reset.
- The package remains generic: it does not provide assistant-specific UI widgets, but it must make assistant-style real-time screens ergonomic to build.

## Standalone Example Coverage Contract

- The release must include a new standalone example at `examples/react-ai-chat` that uses `@kalamdb/react` as the primary live UI integration surface.
- That example must demonstrate conversation selection and creation, conversation-scoped history loading, multiple file uploads per user message, typing indicators, streamed assistant replies, visible tool-call activity, and message-level cancel or edit actions.
- The example must follow the same general browser-plus-topic-agent topology as `examples/chat-with-ai` while validating the richer chat-application workflow required by this feature.