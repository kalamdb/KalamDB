import type { Table } from 'drizzle-orm';
import type { LiveQueryProps } from '../types.js';
import { useLiveQuery } from '../hooks/useLiveQuery.js';

export function LiveQuery<TTable extends Table>(props: LiveQueryProps<TTable>) {
  const { children, ...options } = props;
  const context = useLiveQuery(options as never);
  return <>{children(context as never)}</>;
}