import React from 'react';
import { act, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { bigint, text, timestamp } from 'drizzle-orm/pg-core';
import { desc, eq } from 'drizzle-orm';
import { kTable } from '@kalamdb/orm';
import { useLiveQueries, useLiveSelection } from '../src/index.js';
import { createMockKalamClient, renderWithKalam } from './test-utils.js';

const messages = kTable.user('assistant.messages', {
  id: bigint('id', { mode: 'number' }).primaryKey(),
  threadId: text('thread_id').notNull(),
  body: text('body').notNull(),
});

const toolCalls = kTable.user('assistant.tool_calls', {
  id: bigint('id', { mode: 'number' }).primaryKey(),
  threadId: text('thread_id').notNull(),
  status: text('status').notNull(),
  createdAt: timestamp('created_at', { mode: 'date' }).notNull(),
});

const typing = kTable.user('assistant.typing', {
  id: bigint('id', { mode: 'number' }).primaryKey(),
  threadId: text('thread_id').notNull(),
  userName: text('user_name').notNull(),
});

const approvals = kTable.user('assistant.approvals', {
  id: bigint('id', { mode: 'number' }).primaryKey(),
  threadId: text('thread_id').notNull(),
  status: text('status').notNull(),
});

function AssistantWorkflow() {
  const context = useLiveQueries({
    queries: {
      messages: { table: messages, where: (table) => eq(table.threadId, 't1') },
      toolCalls: {
        table: toolCalls,
        where: (table) => eq(table.threadId, 't1'),
        orderBy: (table) => desc(table.createdAt),
      },
      typing: { table: typing, where: (table) => eq(table.threadId, 't1') },
      approvals: { table: approvals, where: (table) => eq(table.threadId, 't1') },
    },
  });
  const selected = useLiveSelection(context, (live) => ({
    pendingApprovals: live.approvals.rows.filter((row) => row.status === 'pending'),
    activeToolCalls: live.toolCalls.rows.filter((row) => row.status !== 'completed'),
    typingUsers: live.typing.rows.map((row) => row.userName),
  }));

  return (
    <div>
      <span data-testid="pending">{selected.pendingApprovals.length}</span>
      <span data-testid="tools">{selected.activeToolCalls.length}</span>
      <span data-testid="typing">{selected.typingUsers.join(',')}</span>
      <button onClick={() => void context.update(approvals, 'approval-1').set({ status: 'approved' })}>approve</button>
    </div>
  );
}

describe('assistant workflow composition', () => {
  it('derives approval, typing, and tool state without mirror effects', async () => {
    const client = createMockKalamClient();
    renderWithKalam(<AssistantWorkflow />, client as never);

    await waitFor(() => expect(client.liveCalls).toHaveLength(4));
    act(() => {
      client.liveCalls[0].emit([{ id: 1, thread_id: 't1', body: 'Hi' }]);
      client.liveCalls[1].emit([{ id: 2, thread_id: 't1', status: 'running', created_at: new Date() }]);
      client.liveCalls[2].emit([{ id: 3, thread_id: 't1', user_name: 'Avery' }]);
      client.liveCalls[3].emit([{ id: 4, thread_id: 't1', status: 'pending' }]);
    });

    expect(screen.getByTestId('pending').textContent).toBe('1');
    expect(screen.getByTestId('tools').textContent).toBe('1');
    expect(screen.getByTestId('typing').textContent).toBe('Avery');

    await act(async () => {
      screen.getByRole('button', { name: 'approve' }).click();
    });

    expect(client.updated).toEqual([{ tableName: 'assistant.approvals', rowKey: 'approval-1', patch: { status: 'approved' } }]);
  });
});