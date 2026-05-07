import React, { type ReactElement } from 'react';
import { render, type RenderOptions } from '@testing-library/react';
import {
  LiveQueryController,
  type KalamDBClient,
  type LiveCallback,
  type LiveOptions,
  type LiveQueryControllerOptions,
  type LiveQueryDescriptor,
  type Unsubscribe,
} from '@kalamdb/client';
import { KalamProvider } from '../src/context.js';

type LiveCall = {
  sql: string;
  options: LiveOptions<unknown>;
  emit: LiveCallback<unknown>;
};

export interface MockKalamClient extends Pick<KalamDBClient, 'createLiveQueryController' | 'insert' | 'update' | 'delete' | 'live'> {
  liveCalls: LiveCall[];
  inserted: Array<{ tableName: string; row: Record<string, unknown> }>;
  updated: Array<{ tableName: string; rowKey: string; patch: Record<string, unknown> }>;
  deleted: Array<{ tableName: string; rowKey: string }>;
}

export function createMockKalamClient(): MockKalamClient {
  return {
    liveCalls: [],
    inserted: [],
    updated: [],
    deleted: [],
    createLiveQueryController<TRow>(descriptor: LiveQueryDescriptor<TRow>, options: LiveQueryControllerOptions<TRow> = {}) {
      return new LiveQueryController(this as unknown as KalamDBClient, descriptor, options);
    },
    async live(sql: string, callback: LiveCallback<unknown>, options: LiveOptions<unknown> = {}): Promise<Unsubscribe> {
      this.liveCalls.push({
        sql,
        options,
        emit: (rows) => callback(options.mapRow ? rows.map((row) => options.mapRow?.(row as never)) : rows),
      });
      return async () => undefined;
    },
    async insert(tableName: string, row: Record<string, unknown>) {
      this.inserted.push({ tableName, row });
      return { status: 'success', results: [] } as never;
    },
    async update(tableName: string, rowKey: string, patch: Record<string, unknown>) {
      this.updated.push({ tableName, rowKey, patch });
      return undefined;
    },
    async delete(tableName: string, rowKey: string) {
      this.deleted.push({ tableName, rowKey });
      return undefined;
    },
  };
}

export function renderWithKalam(
  element: ReactElement,
  client: KalamDBClient = createMockKalamClient() as unknown as KalamDBClient,
  options?: Omit<RenderOptions, 'wrapper'>,
) {
  return {
    client,
    ...render(<KalamProvider client={client}>{element}</KalamProvider>, options),
  };
}