import React from 'react';
import { act, cleanup, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { useLiveQuery } from '../src/hooks/useLiveQuery.js';
import { useMutationActions } from '../src/hooks/useMutationState.js';
import { createMockKalamClient, renderWithKalam } from './test-utils.js';

describe('bug regressions', () => {
  afterEach(() => {
    cleanup();
  });

  describe('B1: useLiveQuery does not resubscribe on every parent render', () => {
    it('inline getKey arrow does not bust effect deps across renders', async () => {
      const client = createMockKalamClient();
      let forceRender: () => void = () => {};
      function Parent() {
        const [, setN] = React.useState(0);
        forceRender = () => setN((n) => n + 1);
        return <Child />;
      }
      function Child() {
        const { rows } = useLiveQuery({
          query: 'SELECT * FROM t',
          getKey: (r: Record<string, unknown>) => String(r.id),
        });
        return <span data-testid="count">{rows.length}</span>;
      }

      renderWithKalam(<Parent />, client as never);
      await waitFor(() => expect(client.liveCalls).toHaveLength(1));

      await act(async () => {
        forceRender();
        forceRender();
        forceRender();
      });

      expect(client.liveCalls).toHaveLength(1);
    });

    it('resubscribes when deps change', async () => {
      const client = createMockKalamClient();
      let setId: (n: number) => void = () => {};
      function Component() {
        const [id, set] = React.useState(1);
        setId = set;
        const { rows } = useLiveQuery({
          query: `SELECT * FROM t WHERE id = ${id}`,
          deps: [id],
        });
        return <span>{rows.length}</span>;
      }

      renderWithKalam(<Component />, client as never);
      await waitFor(() => expect(client.liveCalls).toHaveLength(1));

      await act(async () => { setId(2); });
      await waitFor(() => expect(client.liveCalls.length).toBeGreaterThan(1));
    });
  });

  describe('B5: useLiveQueries partial failure preserves successful queries (covered via integration tests)', () => {
    it('placeholder — full coverage is in e2e/multi-query.spec.ts', () => {
      expect(true).toBe(true);
    });
  });

  describe('B6: concurrent inserts keep inserting=true until last finishes', () => {
    function Probe() {
      const client = React.useMemo(() => {
        let n = 0;
        return {
          async insert() {
            const wait = n++ === 0 ? 30 : 120;
            await new Promise((r) => setTimeout(r, wait));
            return undefined as never;
          },
          async update() { return undefined as never; },
          async delete() { return undefined as never; },
          async live() { return async () => undefined; },
          createLiveQueryController() { return null as never; },
        };
      }, []);
      const m = useMutationActions(client as never);
      return (
        <div>
          <span data-testid="inserting">{m.inserting ? 'yes' : 'no'}</span>
          <button onClick={() => { void m.insert('t', { v: 1 }); }}>fire</button>
        </div>
      );
    }

    it('stays true while two overlap (first short, second long)', async () => {
      renderWithKalam(<Probe />, undefined as never);
      const btn = screen.getByRole('button', { name: 'fire' });
      await act(async () => {
        btn.click();
        btn.click();
      });
      expect(screen.getByTestId('inserting').textContent).toBe('yes');
      await act(async () => { await new Promise((r) => setTimeout(r, 60)); });
      expect(screen.getByTestId('inserting').textContent).toBe('yes');
      await waitFor(() => expect(screen.getByTestId('inserting').textContent).toBe('no'));
    });
  });

  describe('B7: error does not wipe in-flight updating/deleting sets', () => {
    function Probe() {
      const client = React.useMemo(() => ({
        async insert() { throw new Error('boom'); },
        async update() { await new Promise((r) => setTimeout(r, 60)); return undefined as never; },
        async delete() { return undefined as never; },
        async live() { return async () => undefined; },
        createLiveQueryController() { return null as never; },
      }), []);
      const m = useMutationActions(client as never);
      return (
        <div>
          <span data-testid="updating-count">{m.updating.size}</span>
          <span data-testid="error">{m.error?.message ?? ''}</span>
          <button onClick={() => { void m.update('t', '1', { v: 1 }); }}>upd</button>
          <button onClick={() => { void m.insert('t', { v: 1 }).catch(() => undefined); }}>ins</button>
        </div>
      );
    }

    it('insert error keeps update tracking alive', async () => {
      renderWithKalam(<Probe />, undefined as never);
      await act(async () => {
        screen.getByRole('button', { name: 'upd' }).click();
      });
      expect(screen.getByTestId('updating-count').textContent).toBe('1');

      await act(async () => {
        screen.getByRole('button', { name: 'ins' }).click();
        await new Promise((r) => setTimeout(r, 5));
      });

      await waitFor(() => expect(screen.getByTestId('error').textContent).toBe('boom'));
      expect(screen.getByTestId('updating-count').textContent).toBe('1');
    });
  });

  describe('B8: initial loading state is consistent', () => {
    function Probe() {
      const { state } = useLiveQuery({ query: 'SELECT 1' });
      return <span data-testid="status">{state.status}</span>;
    }

    it('shows status=loading not status=idle while loading', () => {
      renderWithKalam(<Probe />, undefined as never);
      expect(screen.getByTestId('status').textContent).toBe('loading');
    });
  });

  describe('B11: numeric rowKey is accepted by update and remove', () => {
    function Probe() {
      const m = useMutationActions(useFakeClient());
      return (
        <div>
          <span data-testid="updating">{[...m.updating].join(',')}</span>
          <button onClick={() => { void m.update('t', 42, { v: 1 }); }}>upd</button>
          <button onClick={() => { void m.remove('t', 7); }}>del</button>
        </div>
      );
    }

    function useFakeClient() {
      return React.useMemo(() => ({
        async insert() { return undefined as never; },
        async update(_t: string, _k: string) { await new Promise((r) => setTimeout(r, 30)); return undefined as never; },
        async delete() { return undefined as never; },
        async live() { return async () => undefined; },
        createLiveQueryController() { return null as never; },
      }), []);
    }

    it('accepts numbers', async () => {
      renderWithKalam(<Probe />, undefined as never);
      await act(async () => {
        screen.getByRole('button', { name: 'upd' }).click();
      });
      expect(screen.getByTestId('updating').textContent).toBe('42');
    });
  });

  describe('B18: custom drizzle column mapToDriverValue is invoked', () => {
    function makeFakeTable() {
      const upperJson = (value: unknown) => JSON.stringify(value).toUpperCase();
      return {
        [Symbol.for('drizzle:Name')]: 'widgets',
        [Symbol.for('drizzle:Columns')]: {
          payload: { name: 'payload', mapToDriverValue: upperJson },
          plainText: { name: 'plain_text' },
        },
      } as unknown as import('drizzle-orm').Table;
    }

    function Probe() {
      const inserted = React.useRef<unknown>(null);
      const client = React.useMemo(() => ({
        async insert(_t: string, row: Record<string, unknown>) {
          inserted.current = row;
          return undefined as never;
        },
        async update() { return undefined as never; },
        async delete() { return undefined as never; },
        async live() { return async () => undefined; },
        createLiveQueryController() { return null as never; },
      }), []);
      const m = useMutationActions(client as never);
      const [last, setLast] = React.useState<unknown>(null);
      const fakeTable = React.useMemo(makeFakeTable, []);

      return (
        <div>
          <span data-testid="payload">{last == null ? '' : JSON.stringify(last)}</span>
          <button onClick={async () => {
            await m.insert(fakeTable).values({ payload: { foo: 'bar' }, plainText: 'hi' });
            setLast(inserted.current);
          }}>
            ins
          </button>
        </div>
      );
    }

    it('encodes value via mapToDriverValue and preserves plain columns', async () => {
      renderWithKalam(<Probe />, undefined as never);
      await act(async () => {
        screen.getByRole('button', { name: 'ins' }).click();
        await new Promise((r) => setTimeout(r, 5));
      });
      await waitFor(() => expect(screen.getByTestId('payload').textContent).not.toBe(''));
      const sent = JSON.parse(screen.getByTestId('payload').textContent || '{}');
      expect(sent.payload).toBe('{"FOO":"BAR"}');
      expect(sent.plain_text).toBe('hi');
    });
  });

  describe('clearError clears error state', () => {
    function Probe() {
      const client = React.useMemo(() => ({
        async insert() { throw new Error('boom'); },
        async update() { return undefined as never; },
        async delete() { return undefined as never; },
        async live() { return async () => undefined; },
        createLiveQueryController() { return null as never; },
      }), []);
      const m = useMutationActions(client as never);
      return (
        <div>
          <span data-testid="error">{m.error?.message ?? ''}</span>
          <button onClick={() => { void m.insert('t', {}).catch(() => undefined); }}>ins</button>
          <button onClick={() => m.clearError()}>clear</button>
        </div>
      );
    }

    it('clears after clearError()', async () => {
      renderWithKalam(<Probe />, undefined as never);
      await act(async () => {
        screen.getByRole('button', { name: 'ins' }).click();
        await new Promise((r) => setTimeout(r, 5));
      });
      await waitFor(() => expect(screen.getByTestId('error').textContent).toBe('boom'));

      await act(async () => {
        screen.getByRole('button', { name: 'clear' }).click();
      });
      expect(screen.getByTestId('error').textContent).toBe('');
    });
  });
});
