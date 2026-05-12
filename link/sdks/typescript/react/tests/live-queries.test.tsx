import React from 'react';
import { act, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { bigint, text, timestamp } from 'drizzle-orm/pg-core';
import { asc, eq } from 'drizzle-orm';
import { kTable } from '@kalamdb/orm';
import { LiveQueries, useLiveQueries } from '../src/index.js';
import { createMockKalamClient, renderWithKalam } from './test-utils.js';

const messages = kTable.user('chat.messages', {
  id: bigint('id', { mode: 'number' }).primaryKey(),
  conversationId: text('conversation_id').notNull(),
  body: text('body').notNull(),
  createdAt: timestamp('created_at', { mode: 'date' }).notNull(),
});

const typing = kTable.user('chat.typing', {
  id: bigint('id', { mode: 'number' }).primaryKey(),
  conversationId: text('conversation_id').notNull(),
  userName: text('user_name').notNull(),
});

const queries = {
  messages: {
    table: messages,
    where: (table: typeof messages) => eq(table.conversationId, 'c1'),
    orderBy: (table: typeof messages) => asc(table.createdAt),
  },
  typing: {
    table: typing,
    where: (table: typeof typing) => eq(table.conversationId, 'c1'),
  },
};

function ChatSurface() {
  const chat = useLiveQueries({ queries });

  return (
    <div>
      <span data-testid="loading">{chat.state.loading ? 'loading' : 'ready'}</span>
      <span data-testid="messages">{chat.messages.rows.length}</span>
      <span data-testid="typing">{chat.typing.rows.map((row) => row.userName).join(',')}</span>
      <button onClick={() => void chat.insert(typing).values({ conversationId: 'c1', userName: 'Jamal' })}>type</button>
      <button onClick={() => void chat.update(typing, '2').set({ userName: 'Robin' })}>rename</button>
    </div>
  );
}

describe('useLiveQueries', () => {
  it('combines named live datasets and routes mutations through typed targets', async () => {
    const client = createMockKalamClient();
    renderWithKalam(<ChatSurface />, client as never);

    await waitFor(() => expect(client.liveCalls).toHaveLength(2));
    act(() => {
      client.liveCalls[0].emit([{ id: 1, conversation_id: 'c1', body: 'Hello', created_at: new Date() }]);
      client.liveCalls[1].emit([{ id: 2, conversation_id: 'c1', user_name: 'Taylor' }]);
    });

    expect(screen.getByTestId('loading').textContent).toBe('ready');
    expect(screen.getByTestId('messages').textContent).toBe('1');
    expect(screen.getByTestId('typing').textContent).toBe('Taylor');

    await act(async () => {
      screen.getByRole('button', { name: 'type' }).click();
    });

    await act(async () => {
      screen.getByRole('button', { name: 'rename' }).click();
    });

    expect(client.inserted).toEqual([{ tableName: 'chat.typing', row: { conversation_id: 'c1', user_name: 'Jamal' } }]);
    expect(client.updated).toEqual([{ tableName: 'chat.typing', rowKey: '2', patch: { user_name: 'Robin' } }]);
  });
});

describe('LiveQueries', () => {
  it('renders the component wrapper over the multi-query hook', async () => {
    const client = createMockKalamClient();
    renderWithKalam(
      <LiveQueries queries={queries}>
        {({ messages: messageResult }) => <span data-testid="wrapper-count">{messageResult.rows.length}</span>}
      </LiveQueries>,
      client as never,
    );

    await waitFor(() => expect(client.liveCalls).toHaveLength(2));
    act(() => {
      client.liveCalls[0].emit([{ id: 1, conversation_id: 'c1', body: 'Hello', created_at: new Date() }]);
    });

    expect(screen.getByTestId('wrapper-count').textContent).toBe('1');
  });
});