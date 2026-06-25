import { mkdirSync } from 'node:fs';
import { dirname } from 'node:path';
import Database from 'better-sqlite3';
import { drizzle } from 'drizzle-orm/better-sqlite3';
import { local_context_files, pending_uploads } from '../models/schema.local.js';

export type LocalDb = ReturnType<typeof openLocalDb>;

export function openLocalDb(dbPath: string) {
  mkdirSync(dirname(dbPath), { recursive: true });
  const sqlite = new Database(dbPath);
  sqlite.pragma('journal_mode = WAL');
  sqlite.pragma('foreign_keys = ON');

  const db = drizzle(sqlite, { schema: { local_context_files, pending_uploads } });

  sqlite.exec(`
    CREATE TABLE IF NOT EXISTS context_files (
      path TEXT PRIMARY KEY NOT NULL,
      sha256 TEXT NOT NULL,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS pending_uploads (
      path TEXT PRIMARY KEY NOT NULL,
      sha256 TEXT NOT NULL,
      updated_at INTEGER NOT NULL,
      last_error TEXT
    );
  `);

  return db;
}
