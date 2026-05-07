import { useMemo } from 'react';
import type { LiveSelectionSelector } from '../types.js';

export function useLiveSelection<TContext, TSelected>(
  context: TContext,
  selector: LiveSelectionSelector<TContext, TSelected>,
): TSelected {
  return useMemo(() => selector(context), [context, selector]);
}