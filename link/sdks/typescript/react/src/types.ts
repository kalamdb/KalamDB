import type { ReactNode } from 'react';
import type {
  KalamDBClient,
  LiveGetKey,
  LiveQueryControllerSnapshot,
  RowData,
} from '@kalamdb/client';
import type { InferInsertModel, InferSelectModel, Table } from 'drizzle-orm';

/**
 * Structural stand-in for drizzle-orm `SQL` / `SQLWrapper`.
 *
 * Public `where` / `orderBy` callbacks must not name drizzle's own types:
 * `file:` installs of this package keep a nested `node_modules/drizzle-orm`,
 * and TypeScript then treats the app's `SQL` as a different class because
 * `shouldInlineParams` is private.
 */
export type LiveSqlFragment = {
  getSQL(): unknown;
};

export type LiveSqlExpr = LiveSqlFragment | readonly LiveSqlFragment[];

export interface KalamProviderProps {
  client: KalamDBClient;
  children: ReactNode;
}

/**
 * Identifier for a row's primary key. Drizzle/KalamDB primary keys may be
 * either text (e.g. UUIDs, room IDs) or numeric. Mutation APIs accept both.
 */
export type RowKey = string | number;

export interface MutationState {
  inserting: boolean;
  updating: Set<RowKey>;
  deleting: Set<RowKey>;
  error?: Error;
}

export type InsertAction = {
  <TTable extends Table>(table: TTable): {
    values: (row: InferInsertModel<TTable>) => Promise<void>;
  };
  (tableName: string, row: Record<string, unknown>): Promise<void>;
};

export type UpdateAction = {
  <TTable extends Table>(table: TTable, rowKey: RowKey): {
    set: (patch: Partial<InferInsertModel<TTable>>) => Promise<void>;
  };
  (tableName: string, rowKey: RowKey, patch: Record<string, unknown>): Promise<void>;
};

export type RemoveAction = {
  <TTable extends Table>(table: TTable, rowKey: RowKey): Promise<void>;
  (tableName: string, rowKey: RowKey): Promise<void>;
};

export interface MutationActions {
  insert: InsertAction;
  update: UpdateAction;
  remove: RemoveAction;
  /**
   * Clear the most recent mutation error from state. In-flight `updating` /
   * `deleting` tracking is preserved (a failed mutation never affects unrelated
   * pending ones).
   */
  clearError: () => void;
}

export interface SingleLiveQueryContext<TRow> extends MutationActions {
  rows: TRow[];
  state: LiveQueryControllerSnapshot<TRow> & MutationState;
  refetch: () => Promise<void>;
}

export type LiveQueryMode = 'drizzle' | 'sql';

export interface SharedLiveQueryOptions<TRow> {
  client?: KalamDBClient;
  limit?: number;
  getKey?: LiveGetKey<TRow>;
  deps?: readonly unknown[];
}

export interface DrizzleLiveQueryOptions<TTable extends Table, TSelected = SingleLiveQueryContext<InferSelectModel<TTable>>>
  extends SharedLiveQueryOptions<InferSelectModel<TTable>> {
  table: TTable;
  where?: (table: TTable) => LiveSqlFragment;
  orderBy?: (table: TTable) => LiveSqlExpr;
  select?: (context: SingleLiveQueryContext<InferSelectModel<TTable>>) => TSelected;
}

export interface RawSqlLiveQueryOptions<TSelected = SingleLiveQueryContext<RowData>>
  extends SharedLiveQueryOptions<RowData> {
  query: string;
  select?: (context: SingleLiveQueryContext<RowData>) => TSelected;
}

export type UseLiveQueryOptions<TTable extends Table = Table, TSelected = unknown> =
  | DrizzleLiveQueryOptions<TTable, TSelected>
  | RawSqlLiveQueryOptions<TSelected>;

export type DrizzleLiveQueryProps<TTable extends Table> = Omit<DrizzleLiveQueryOptions<TTable>, 'select'> & {
  children: (context: SingleLiveQueryContext<InferSelectModel<TTable>>) => ReactNode;
};

export type RawSqlLiveQueryProps = Omit<RawSqlLiveQueryOptions, 'select'> & {
  children: (context: SingleLiveQueryContext<RowData>) => ReactNode;
};

export type LiveQueryProps<TTable extends Table = Table> = DrizzleLiveQueryProps<TTable> | RawSqlLiveQueryProps;

export interface DrizzleLiveQueryDefinition<TTable extends Table = Table>
  extends Omit<SharedLiveQueryOptions<InferSelectModel<TTable>>, 'client'> {
  table: TTable;
  where?: (table: TTable) => LiveSqlFragment;
  orderBy?: (table: TTable) => LiveSqlExpr;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type LiveQueriesDefinition = Record<string, DrizzleLiveQueryDefinition<any>>;

export type InferLiveQueryRow<TDefinition> =
  TDefinition extends { table: infer TTable extends Table }
    ? InferSelectModel<TTable>
    : RowData;

export interface MultiLiveQueryAggregateState extends MutationState {
  loading: boolean;
  connected: boolean;
  error?: Error;
}

export type MultiLiveQueryContext<TQueries extends LiveQueriesDefinition> = {
  [K in keyof TQueries]: SingleLiveQueryContext<InferLiveQueryRow<TQueries[K]>>;
} & MutationActions & {
  state: MultiLiveQueryAggregateState;
  refetch: () => Promise<void>;
};

export interface UseLiveQueriesOptions<
  TQueries extends LiveQueriesDefinition,
  TSelected = MultiLiveQueryContext<TQueries>,
> {
  client?: KalamDBClient;
  queries: TQueries;
  select?: (context: MultiLiveQueryContext<TQueries>) => TSelected;
  deps?: readonly unknown[];
}

export interface LiveQueriesProps<TQueries extends LiveQueriesDefinition> {
  client?: KalamDBClient;
  queries: TQueries;
  children: (context: MultiLiveQueryContext<TQueries>) => ReactNode;
  deps?: readonly unknown[];
}

export type LiveSelectionSelector<TContext, TSelected> = (context: TContext) => TSelected;
