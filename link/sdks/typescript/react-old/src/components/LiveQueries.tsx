import type { LiveQueriesDefinition, LiveQueriesProps } from '../types.js';
import { useLiveQueries } from '../hooks/useLiveQueries.js';

export function LiveQueries<TQueries extends LiveQueriesDefinition>(props: LiveQueriesProps<TQueries>) {
  const { children, ...options } = props;
  const context = useLiveQueries(options);
  return <>{children(context)}</>;
}