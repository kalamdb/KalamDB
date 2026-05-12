import { expectTypeOf, test } from 'vitest';
import type { ReactElement } from 'react';
import { pgSchema, text, integer, timestamp } from 'drizzle-orm/pg-core';
import { eq, asc } from 'drizzle-orm';
import {
  useLiveQuery,
  useLiveQueries,
  useLiveSelection,
  useMutationActions,
  LiveQuery,
  LiveQueries,
  KalamProvider,
  type MultiLiveQueryContext,
  type SingleLiveQueryContext,
  type RowKey,
  type MutationState,
} from '../src/index.js';
import type { RowData } from '@kalamdb/client';

const e2eSchema = pgSchema('e2e');
const messages = e2eSchema.table('messages', {
  id: text('id').primaryKey(),
  body: text('body').notNull(),
  authorId: integer('author_id'),
  createdAt: timestamp('created_at'),
});

type MessageRow = {
  id: string;
  body: string;
  authorId: number | null;
  createdAt: Date | null;
};

test('useLiveQuery — drizzle mode returns SingleLiveQueryContext<InferSelectModel<T>>', () => {
  const ctx = useLiveQuery({ table: messages });
  expectTypeOf(ctx).toMatchTypeOf<SingleLiveQueryContext<MessageRow>>();
  expectTypeOf(ctx.rows).toMatchTypeOf<MessageRow[]>();
  expectTypeOf<(typeof ctx.rows)[number]>().toMatchTypeOf<MessageRow>();
});

test('useLiveQuery — drizzle mode with select returns TSelected', () => {
  const view = useLiveQuery({
    table: messages,
    select: (c) => ({ count: c.rows.length, first: c.rows[0]?.body ?? '' }),
  });
  expectTypeOf(view).toEqualTypeOf<{ count: number; first: string }>();
});

test('useLiveQuery — sql mode returns SingleLiveQueryContext<RowData>', () => {
  const ctx = useLiveQuery({ query: 'SELECT * FROM e2e.messages' });
  expectTypeOf(ctx).toEqualTypeOf<SingleLiveQueryContext<RowData>>();
  expectTypeOf(ctx.rows).toEqualTypeOf<RowData[]>();
});

test('useLiveQuery — sql mode with select returns TSelected', () => {
  const view = useLiveQuery({
    query: 'SELECT * FROM e2e.messages',
    select: (c) => c.rows.length,
  });
  expectTypeOf(view).toEqualTypeOf<number>();
});

test('useLiveQuery — drizzle where/orderBy callback receives the typed table', () => {
  useLiveQuery({
    table: messages,
    where: (t) => {
      expectTypeOf(t).toEqualTypeOf<typeof messages>();
      return eq(t.body, 'x');
    },
    orderBy: (t) => {
      expectTypeOf(t).toEqualTypeOf<typeof messages>();
      return asc(t.createdAt);
    },
  });
});

test('useLiveQueries — per-key inference for drizzle tables', () => {
  const ctx = useLiveQueries({
    queries: {
      messages: { table: messages },
      others: { table: messages },
    },
  });
  expectTypeOf(ctx.messages.rows).toEqualTypeOf<MessageRow[]>();
  expectTypeOf(ctx.others.rows).toEqualTypeOf<MessageRow[]>();
  expectTypeOf(ctx.state.loading).toEqualTypeOf<boolean>();
  expectTypeOf(ctx.state.connected).toEqualTypeOf<boolean>();
});

test('useLiveQueries — select transform returns TSelected', () => {
  const view = useLiveQueries({
    queries: { m: { table: messages } },
    select: (c) => c.m.rows.map((r) => r.body),
  });
  expectTypeOf(view).toEqualTypeOf<string[]>();
});

test('MultiLiveQueryContext exposes typed mutations', () => {
  type Ctx = MultiLiveQueryContext<{ m: { table: typeof messages } }>;
  type Insert = Ctx['insert'];
  expectTypeOf<Insert>().toBeFunction();
});

test('useMutationActions — insert(table).values(typedRow)', () => {
  const m = useMutationActions({} as never);
  const builder = m.insert(messages);
  expectTypeOf(builder.values).parameter(0).toMatchTypeOf<{
    id: string;
    body: string;
    authorId?: number | null;
    createdAt?: Date | null;
  }>();
  expectTypeOf(builder.values).returns.toEqualTypeOf<Promise<void>>();
});

test('useMutationActions — insert(string, anyRecord) overload', () => {
  const m = useMutationActions({} as never);
  expectTypeOf(m.insert).toBeCallableWith('e2e.messages', { id: 'x' });
});

test('useMutationActions — update(table, rowKey) returns builder', () => {
  const m = useMutationActions({} as never);
  const builder = m.update(messages, 'row-1');
  expectTypeOf(builder.set).parameter(0).toMatchTypeOf<Partial<{
    id: string;
    body: string;
    authorId: number | null;
    createdAt: Date | null;
  }>>();
});

test('useMutationActions — rowKey accepts string | number (B11)', () => {
  const m = useMutationActions({} as never);
  expectTypeOf(m.update).toBeCallableWith(messages, 42);
  expectTypeOf(m.update).toBeCallableWith(messages, 'k');
  expectTypeOf(m.remove).toBeCallableWith(messages, 42);
  expectTypeOf(m.remove).toBeCallableWith('e2e.messages', 7);
});

test('useMutationActions — clearError is exposed and returns void', () => {
  const m = useMutationActions({} as never);
  expectTypeOf(m.clearError).toEqualTypeOf<() => void>();
});

test('MutationState — sets carry RowKey not just string', () => {
  expectTypeOf<MutationState['updating']>().toEqualTypeOf<Set<RowKey>>();
  expectTypeOf<MutationState['deleting']>().toEqualTypeOf<Set<RowKey>>();
  expectTypeOf<MutationState['inserting']>().toEqualTypeOf<boolean>();
});

test('useLiveSelection — selector input matches passed context, output is TSelected', () => {
  const ctx = useLiveQuery({ table: messages });
  const v = useLiveSelection(ctx, (c) => c.rows.length);
  expectTypeOf(v).toEqualTypeOf<number>();
});

test('LiveQuery component — drizzle overload children receives typed rows', () => {
  const _el: ReactElement = (
    <LiveQuery table={messages}>
      {(c) => {
        expectTypeOf(c).toEqualTypeOf<SingleLiveQueryContext<MessageRow>>();
        return null;
      }}
    </LiveQuery>
  );
  expectTypeOf(_el).toEqualTypeOf<ReactElement>();
});

test('LiveQuery component — sql overload children receives RowData rows', () => {
  const _el: ReactElement = (
    <LiveQuery query="SELECT 1">
      {(c) => {
        expectTypeOf(c).toEqualTypeOf<SingleLiveQueryContext<RowData>>();
        return null;
      }}
    </LiveQuery>
  );
  expectTypeOf(_el).toEqualTypeOf<ReactElement>();
});

test('LiveQueries component — children receives typed per-key context', () => {
  const _el: ReactElement = (
    <LiveQueries queries={{ m: { table: messages } }}>
      {(c) => {
        expectTypeOf(c.m.rows).toEqualTypeOf<MessageRow[]>();
        expectTypeOf(c.state.connected).toEqualTypeOf<boolean>();
        return null;
      }}
    </LiveQueries>
  );
  expectTypeOf(_el).toEqualTypeOf<ReactElement>();
});

test('KalamProvider — client prop is required', () => {
  expectTypeOf<Parameters<typeof KalamProvider>[0]>().toHaveProperty('client');
});

test('compile-error guard: rowKey rejects non-string non-number (commented; uncomment to verify)', () => {
  const m = useMutationActions({} as never);
  // @ts-expect-error — boolean is not RowKey
  m.update(messages, true);
  // @ts-expect-error — object is not RowKey
  m.remove(messages, {});
});

test('compile-error guard: LiveQuery rejects passing both query and table', () => {
  // @ts-expect-error — cannot pass both
  const _ = <LiveQuery table={messages} query="SELECT 1">{() => null}</LiveQuery>;
  void _;
});
