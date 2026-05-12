import React from 'react';
import { act, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { LiveQuery } from '../src/components/LiveQuery.js';
import { createMockKalamClient, renderWithKalam } from './test-utils.js';

describe('LiveQuery SQL mode', () => {
  it('normalizes ORDER BY/LIMIT and projects rows client-side', async () => {
    const client = createMockKalamClient();

    renderWithKalam(
      <LiveQuery query="SELECT * FROM chat.messages WHERE room = 1 ORDER BY created_at DESC LIMIT 1">
        {({ rows }) => <span data-testid="first">{rows[0] ? String(rows[0].id) : 'none'}</span>}
      </LiveQuery>,
      client as never,
    );

    await waitFor(() => expect(client.liveCalls).toHaveLength(1));
    expect(client.liveCalls[0].sql).toBe('SELECT * FROM chat.messages WHERE room = 1');

    act(() => {
      client.liveCalls[0].emit([
        { id: 'older', created_at: 1 },
        { id: 'newer', created_at: 2 },
      ]);
    });

    expect(screen.getByTestId('first').textContent).toBe('newer');
  });
});