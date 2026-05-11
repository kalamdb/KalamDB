import type { ReactElement } from 'react';
import type { Table } from 'drizzle-orm';
import type {
  DrizzleLiveQueryProps,
  LiveQueryProps,
  RawSqlLiveQueryProps,
} from '../types.js';
import { useLiveQuery } from '../hooks/useLiveQuery.js';

export function LiveQuery<TTable extends Table>(props: DrizzleLiveQueryProps<TTable>): ReactElement;
export function LiveQuery(props: RawSqlLiveQueryProps): ReactElement;
export function LiveQuery<TTable extends Table>(props: LiveQueryProps<TTable>): ReactElement {
  const { children, ...options } = props as { children: (ctx: unknown) => ReactElement } & Record<string, unknown>;
  const context = useLiveQuery(options as never);
  return <>{children(context)}</>;
}
