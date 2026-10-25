import React from 'react';
import { act, cleanup, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { useMutationActions } from '../src/hooks/useMutationState.js';
import { renderWithKalam } from './test-utils.js';

describe('overlapping same-key mutations', () => {
  afterEach(() => {
    cleanup();
  });

  it('keeps a row pending until the last overlapping update and delete finish', async () => {
    const updateGates: Array<() => void> = [];
    const deleteGates: Array<() => void> = [];

    function Probe() {
      const client = React.useMemo(() => ({
        async insert() {
          return undefined as never;
        },
        async update() {
          await new Promise<void>((resolve) => {
            updateGates.push(resolve);
          });
          return undefined as never;
        },
        async delete() {
          await new Promise<void>((resolve) => {
            deleteGates.push(resolve);
          });
          return undefined as never;
        },
        async live() {
          return async () => undefined;
        },
        createLiveQueryController() {
          return null as never;
        },
      }), []);
      const mutation = useMutationActions(client as never);
      return (
        <div>
          <span data-testid="updating">{mutation.updating.has('row-1') ? 'yes' : 'no'}</span>
          <span data-testid="deleting">{mutation.deleting.has('row-1') ? 'yes' : 'no'}</span>
          <button type="button" onClick={() => { void mutation.update('t', 'row-1', { v: 1 }); }}>update</button>
          <button type="button" onClick={() => { void mutation.remove('t', 'row-1'); }}>delete</button>
        </div>
      );
    }

    renderWithKalam(<Probe />, undefined as never);

    await act(async () => {
      screen.getByRole('button', { name: 'update' }).click();
      screen.getByRole('button', { name: 'update' }).click();
    });
    expect(updateGates).toHaveLength(2);
    expect(screen.getByTestId('updating').textContent).toBe('yes');

    await act(async () => {
      updateGates[0]?.();
    });
    expect(screen.getByTestId('updating').textContent).toBe('yes');

    await act(async () => {
      updateGates[1]?.();
    });
    await waitFor(() => expect(screen.getByTestId('updating').textContent).toBe('no'));

    await act(async () => {
      screen.getByRole('button', { name: 'delete' }).click();
      screen.getByRole('button', { name: 'delete' }).click();
    });
    expect(deleteGates).toHaveLength(2);
    expect(screen.getByTestId('deleting').textContent).toBe('yes');

    await act(async () => {
      deleteGates[0]?.();
    });
    expect(screen.getByTestId('deleting').textContent).toBe('yes');

    await act(async () => {
      deleteGates[1]?.();
    });
    await waitFor(() => expect(screen.getByTestId('deleting').textContent).toBe('no'));
  });
});
