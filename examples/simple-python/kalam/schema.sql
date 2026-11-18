-- schema.sql is the single source of truth for this project's tables.
-- Run `kalam schema gen --languages python` to regenerate generated/kalam.py.

CREATE TYPE note_state AS ENUM ('draft', 'published');

CREATE SHARED TABLE notes (
  id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
  author TEXT NOT NULL,
  body TEXT NOT NULL,
  state note_state NOT NULL,
  tags TEXT[],
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
);
