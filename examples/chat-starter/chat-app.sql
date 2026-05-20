-- KalamDB chat starter schema
-- Run via `npm run setup` (or paste into the SQL editor).
--
-- Storage classes used here:
--   * USER tables — per-user-partitioned. Reads/writes for a given user
--     only see that user's rows. Used for everything durable: conversations,
--     messages, approvals, tasks. Required on chat.tasks so the
--     ON-INSERT-sourced topic carries `user` metadata (runConsumer rejects
--     events without it).
--   * STREAM table — fast write-optimized storage with a TTL. Used for
--     typing_tokens: they're produced at high frequency, consumed within
--     seconds by the UI's live query, and never need long-term retention.
--     TTL handles cleanup so the agent doesn't need a `clearTypingTokens`
--     pass after every turn.
--   * SHARED table — readable by every user. Used for chat.docs (the RAG
--     corpus is global knowledge).

-- KalamDB doesn't currently accept `DROP TOPIC IF EXISTS` (it parses IF as
-- the topic name). The setup script tolerates the "Not found" error on a
-- fresh database — see scripts/setup.ts for the swallow logic.
DROP TOPIC chat.approval_resolutions;
DROP TOPIC chat.task_cancels;
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

-- STREAM table: write-optimized, TTL-expired. The agent INSERTs token deltas
-- here at LLM streaming speed and the UI live-queries them. After the
-- message finalizes, the rows are noise — letting KalamDB's TTL sweep
-- them costs less IO than a per-turn DELETE pass.
CREATE TABLE chat.typing_tokens (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL,
  message_id       TEXT NOT NULL,
  body             TEXT NOT NULL,
  seq              INTEGER NOT NULL,
  created_at       TIMESTAMP NOT NULL
) WITH (TYPE = 'STREAM', TTL_SECONDS = 120);

-- Human-in-the-loop checkpoints. The agent inserts a row, waits on the
-- chat.approval_resolutions topic (sourced ON UPDATE WHERE status flips),
-- and proceeds when the UI flips status to 'approved' / 'rejected'.
CREATE TABLE chat.approvals (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL,
  message_id       TEXT NOT NULL,
  question         TEXT NOT NULL,
  status           TEXT NOT NULL,         -- 'pending' | 'approved' | 'rejected'
  created_at       TIMESTAMP NOT NULL,
  resolved_at      TIMESTAMP
) WITH (TYPE = 'USER');

-- One row per assistant turn. The Stop button flips is_cancelled=true; the
-- agent reacts to the chat.task_cancels topic (sourced ON UPDATE WHERE
-- is_cancelled = true) and aborts the matching in-flight task.
CREATE TABLE chat.tasks (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL,
  message_id       TEXT NOT NULL,
  is_cancelled     BOOLEAN NOT NULL,
  started_at       TIMESTAMP NOT NULL,
  finished_at      TIMESTAMP
) WITH (TYPE = 'USER');

-- ----------------------------------------------------------------------------
-- Topics
-- ----------------------------------------------------------------------------
-- New tasks → agent work queue. ON INSERT means each freshly-inserted task
-- row becomes a topic event the agent's runConsumer picks up.
CREATE TOPIC chat.task_events;
ALTER TOPIC chat.task_events ADD SOURCE chat.tasks ON INSERT;

-- Stop clicks → cancel notifications. Instead of opening a per-task live()
-- subscription (which doesn't scale and can't be wrapped in EXECUTE AS USER),
-- the agent runs one consumer on this topic and aborts the matching local
-- AbortController for every cancel event across all users.
CREATE TOPIC chat.task_cancels;
ALTER TOPIC chat.task_cancels ADD SOURCE chat.tasks ON UPDATE WHERE is_cancelled = true;

-- Approval clicks → resolution events. The agent registers a pending
-- callback per request_approval invocation and resolves it when the
-- matching topic event arrives. Same scalability win as task_cancels.
CREATE TOPIC chat.approval_resolutions;
ALTER TOPIC chat.approval_resolutions ADD SOURCE chat.approvals
  ON UPDATE WHERE status IN ('approved', 'rejected');

-- ----------------------------------------------------------------------------
-- RAG corpus
-- ----------------------------------------------------------------------------
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
