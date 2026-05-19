-- KalamDB chat starter schema
-- Run via `npm run setup` (or paste into the SQL editor).
--
-- All tables are TYPE='USER' to match the kTable.user(...) annotation in
-- src/schema.ts (which expects the standard user system columns: _seq,
-- _deleted, _commit_seq). TYPE='USER' is also required on chat.tasks so the
-- ON INSERT-sourced topic carries user metadata — runConsumer rejects events
-- without it.

-- KalamDB doesn't currently accept `DROP TOPIC IF EXISTS` (it parses IF as
-- the topic name). The setup script tolerates the "Not found" error on a
-- fresh database — see scripts/setup.ts for the swallow logic.
DROP TOPIC chat.task_events;
DROP NAMESPACE IF EXISTS chat;
CREATE NAMESPACE chat;

CREATE TABLE chat.conversations (
  id          TEXT PRIMARY KEY,
  title       TEXT NOT NULL,
  created_at  TIMESTAMP NOT NULL,
  updated_at  TIMESTAMP NOT NULL
) WITH (TYPE = 'USER');

CREATE TABLE chat.messages (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL,
  role             TEXT NOT NULL,         -- 'user' | 'assistant' | 'system'
  body             TEXT NOT NULL,
  status           TEXT NOT NULL,         -- 'pending' | 'streaming' | 'final' | 'cancelled' | 'error'
  created_at       TIMESTAMP NOT NULL,
  updated_at       TIMESTAMP NOT NULL
) WITH (TYPE = 'USER');

-- Cleared by the agent when a message reaches a terminal status.
CREATE TABLE chat.typing_tokens (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL,
  message_id       TEXT NOT NULL,
  body             TEXT NOT NULL,
  seq              INTEGER NOT NULL,
  created_at       TIMESTAMP NOT NULL
) WITH (TYPE = 'USER');

-- Human-in-the-loop checkpoints the agent waits on.
CREATE TABLE chat.approvals (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL,
  message_id       TEXT NOT NULL,
  question         TEXT NOT NULL,
  status           TEXT NOT NULL,         -- 'pending' | 'approved' | 'rejected'
  created_at       TIMESTAMP NOT NULL,
  resolved_at      TIMESTAMP
) WITH (TYPE = 'USER');

-- The agent watches its own task row; Stop button = UPDATE setting
-- is_cancelled=true. Sourced into chat.task_events on INSERT so a fresh
-- agent process picks up new work via runConsumer (Kafka-shaped work queue).
CREATE TABLE chat.tasks (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL,
  message_id       TEXT NOT NULL,
  is_cancelled     BOOLEAN NOT NULL,
  started_at       TIMESTAMP NOT NULL,
  finished_at      TIMESTAMP
) WITH (TYPE = 'USER');

CREATE TOPIC chat.task_events;
ALTER TOPIC chat.task_events ADD SOURCE chat.tasks ON INSERT;

-- Knowledge base for RAG. SHARED (not USER) because the docs are global
-- knowledge — every conversation reads from the same corpus. The
-- EMBEDDING(384) column stores per-document vectors; the COSINE index
-- makes nearest-neighbour search efficient at scale.
CREATE SHARED TABLE chat.docs (
  id          TEXT PRIMARY KEY,
  title       TEXT NOT NULL,
  body        TEXT NOT NULL,
  source      TEXT,
  embedding   EMBEDDING(384),
  created_at  TIMESTAMP NOT NULL
);

ALTER TABLE chat.docs CREATE INDEX embedding USING COSINE;

-- SHARED tables default to ACCESS LEVEL Private — only the table owner
-- (root) can read them. The whole point of chat.docs is that EVERY user
-- can use it as a RAG corpus, so flip it to Public.
ALTER TABLE chat.docs SET ACCESS LEVEL PUBLIC;

-- ---------------------------------------------------------------------------
-- Demo users for the multi-tenant story.
-- ---------------------------------------------------------------------------
-- Three KalamDB users let the starter demonstrate per-user data isolation
-- without dragging in a real auth system. The sign-in dropdown in the UI
-- lets you switch between them; the backend mints each user's KalamDB token
-- with the matching password.
--
-- The passwords below are SHARED — anyone who picks "alice" from the dropdown
-- becomes alice. That's fine for a local demo (open two browser tabs to
-- prove isolation works) and a HARD STOP for any real deployment. See the
-- README's "Plugging in real auth" section for the swap recipe.
--
-- KalamDB doesn't parse IF NOT EXISTS on CREATE USER, and DROP USER is a
-- soft-delete (re-creating with the same name is rejected). So setup.ts is
-- the one that handles re-runs: it swallows "User '<x>' already exists"
-- errors on these statements (see the swallow logic in scripts/setup.ts).
CREATE USER alice  WITH PASSWORD 'demo-alice-pw'  ROLE user;
CREATE USER bob    WITH PASSWORD 'demo-bob-pw'    ROLE user;
CREATE USER carol  WITH PASSWORD 'demo-carol-pw'  ROLE user;
