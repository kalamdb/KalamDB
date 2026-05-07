// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { LiveQueryController, type KalamDBClient, type LiveCallback, type LiveOptions, type LiveQueryDescriptor } from '@kalamdb/client';
import { ReactLiveQueryDemo } from './ReactLiveQueryDemo';

const mockClient = createMockClient();

vi.mock('@/lib/kalam-client', () => ({
  getClient: () => mockClient,
}));

describe('ReactLiveQueryDemo', () => {
  afterEach(() => {
    cleanup();
    mockClient.liveCalls.length = 0;
  });

  it('opens React live query streams through the Admin UI client', async () => {
    render(<ReactLiveQueryDemo />);

    await waitFor(() => expect(mockClient.liveCalls.length).toBeGreaterThanOrEqual(4));
    expect(screen.getByText('Component LiveQuery')).toBeTruthy();
    expect(screen.getByText('Hook LiveQueries')).toBeTruthy();
  });
});

function createMockClient() {
  const client = {
    liveCalls: [] as Array<{ sql: string; callback: LiveCallback<unknown>; options: LiveOptions<unknown> }>,
    createLiveQueryController<TRow>(descriptor: LiveQueryDescriptor<TRow>) {
      return new LiveQueryController(this as unknown as KalamDBClient, descriptor);
    },
    async live(sql: string, callback: LiveCallback<unknown>, options: LiveOptions<unknown> = {}) {
      this.liveCalls.push({ sql, callback, options });
      callback([]);
      return async () => undefined;
    },
    async insert() {
      return { status: 'success', results: [] } as never;
    },
    async update() {
      return undefined;
    },
    async delete() {
      return undefined;
    },
  };

  return client;
}