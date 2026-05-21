import { KalamCellValue } from '../cell_value.js';

export type LiveProjectionDirection = 'asc' | 'desc';

export interface LiveProjectionOrder {
  column: string;
  direction: LiveProjectionDirection;
}

export interface LiveProjectionPlan<TRow = unknown> {
  orderBy?: LiveProjectionOrder[];
  limit?: number;
  projectRow?: (row: TRow) => TRow;
}

export function projectLiveRows<TRow>(rows: readonly TRow[], plan: LiveProjectionPlan<TRow> = {}): TRow[] {
  if (!plan.projectRow && !plan.orderBy?.length && typeof plan.limit !== 'number') {
    return Array.isArray(rows) ? rows as TRow[] : [...rows];
  }

  const projected = plan.projectRow ? rows.map(plan.projectRow) : [...rows];

  if (plan.orderBy?.length) {
    projected.sort((left, right) => compareRows(left, right, plan.orderBy ?? []));
  }

  if (typeof plan.limit === 'number' && plan.limit >= 0 && projected.length > plan.limit) {
    return projected.slice(0, plan.limit);
  }

  return projected;
}

export function parseLiveOrderBy(orderBySql: string): LiveProjectionOrder[] {
  const trimmed = orderBySql.trim();
  if (!trimmed) {
    return [];
  }

  return trimmed.split(',').map((part) => {
    const match = part.trim().match(/^(?:(?:"?[A-Za-z_][\w$]*"?)\.)*"?([A-Za-z_][\w$]*)"?(?:\s+(asc|desc))?$/i);
    if (!match) {
      throw new Error(`Unsupported ORDER BY expression for live SQL: ${part.trim()}`);
    }

    return {
      column: match[1],
      direction: (match[2]?.toLowerCase() === 'desc' ? 'desc' : 'asc') as LiveProjectionDirection,
    };
  });
}

function compareRows<TRow>(left: TRow, right: TRow, orderBy: LiveProjectionOrder[]): number {
  for (const order of orderBy) {
    const result = compareProjectionValues(
      rowValue(left, order.column),
      rowValue(right, order.column),
    );

    if (result !== 0) {
      return order.direction === 'desc' ? -result : result;
    }
  }

  return 0;
}

function rowValue(row: unknown, column: string): unknown {
  if (!row || typeof row !== 'object') {
    return undefined;
  }

  const value = (row as Record<string, unknown>)[column];
  if (value instanceof KalamCellValue) {
    return value.toJson();
  }

  return value;
}

function compareProjectionValues(left: unknown, right: unknown): number {
  if (left === right) {
    return 0;
  }
  if (left === null || left === undefined) {
    return 1;
  }
  if (right === null || right === undefined) {
    return -1;
  }

  if (typeof left === 'number' && typeof right === 'number') {
    return left - right;
  }
  if (typeof left === 'bigint' && typeof right === 'bigint') {
    return left < right ? -1 : 1;
  }

  return String(left).localeCompare(String(right));
}