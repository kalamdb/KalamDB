import React from 'react';
import { act, cleanup, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { useLiveQuery } from '../src/hooks/useLiveQuery.js';
import { createMockKalamClient, renderWithKalam } from './test-utils.js';

function Messages() {
  const { rows, state, insert, remove } = useLiveQuery({
    query: 'SELECT * FROM chat.messages WHERE room = 1',
  });

  return (
    <div>
      <span data-testid="status">{state.loading ? 'loading' : 'ready'}</span>
      <span data-testid="deleting">{state.deleting.size > 0 ? 'deleting' : 'idle'}</span>
      <span data-testid="count">{rows.length}</span>
      <button onClick={() => void insert('chat.messages', { room: 1, body: 'Hello' })}>send</button>
      <button onClick={() => void remove('chat.messages', '1')}>remove</button>
    </div>
  );
}

describe('useLiveQuery', () => {
  afterEach(() => {
    cleanup();
  });

  it('renders raw rows and exposes mutation actions', async () => {
    const client = createMockKalamClient();
    renderWithKalam(<Messages />, client as never);

    await waitFor(() => expect(client.liveCalls).toHaveLength(1));
    act(() => {
      client.liveCalls[0].emit([{ id: '1', body: 'Hello' }]);
    });

    expect(screen.getByTestId('status').textContent).toBe('ready');
    expect(screen.getByTestId('count').textContent).toBe('1');

    await act(async () => {
      screen.getByRole('button', { name: 'send' }).click();
    });

    expect(client.inserted).toEqual([{ tableName: 'chat.messages', row: { room: 1, body: 'Hello' } }]);
  });

  it('keeps delete mutation state visible while remove is pending', async () => {
    const client = createMockKalamClient();
    let finishDelete: (() => void) | undefined;
    client.delete = async (tableName: string, rowKey: string) => new Promise<void>((resolve) => {
      client.deleted.push({ tableName, rowKey });
      finishDelete = resolve;
    });

    renderWithKalam(<Messages />, client as never);
    await waitFor(() => expect(client.liveCalls).toHaveLength(1));

    await act(async () => {
      screen.getByRole('button', { name: 'remove' }).click();
    });

    expect(screen.getByTestId('deleting').textContent).toBe('deleting');

    await act(async () => {
      finishDelete?.();
      await Promise.resolve();
    });

    await waitFor(() => expect(screen.getByTestId('deleting').textContent).toBe('idle'));
    expect(client.deleted).toEqual([{ tableName: 'chat.messages', rowKey: '1' }]);
  });
});