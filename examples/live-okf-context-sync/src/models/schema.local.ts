import { integer, sqliteTable, text } from 'drizzle-orm/sqlite-core';

/**
 * Local SQLite mirror of `context_files` metadata.
 * `sha256` caches the last known content hash (from `file_ref.sha256` on pull
 * or computed locally on push). File bytes live on disk beside `sync.db`.
 */
export const local_context_files = sqliteTable('context_files', {
  path: text('path').primaryKey(),
  sha256: text('sha256').notNull(),
  created_at: integer('created_at', { mode: 'timestamp' }).notNull(),
  updated_at: integer('updated_at', { mode: 'timestamp' }).notNull(),
});

export const pending_uploads = sqliteTable('pending_uploads', {
  path: text('path').primaryKey(),
  sha256: text('sha256').notNull(),
  updated_at: integer('updated_at', { mode: 'timestamp' }).notNull(),
  last_error: text('last_error'),
});

export type LocalContextFile = typeof local_context_files.$inferSelect;
export type PendingUpload = typeof pending_uploads.$inferSelect;

/** Shared metadata shape for KalamDB rows and local SQLite rows. */
export type SyncFileRecord = {
  path: string;
  sha256: string;
  created_at: Date;
  updated_at: Date;
};

export function toSyncFileRecord(input: {
  path: string;
  sha256: string;
  created_at: Date | null;
  updated_at: Date | null;
}): SyncFileRecord {
  const now = new Date();
  return {
    path: input.path,
    sha256: input.sha256,
    created_at: input.created_at ?? now,
    updated_at: input.updated_at ?? now,
  };
}
