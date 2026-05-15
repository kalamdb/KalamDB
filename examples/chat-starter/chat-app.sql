-- KalamDB chat starter schema
-- Run via `npm run setup` (or paste into the SQL editor).

DROP TOPIC chat.task_events;
DROP NAMESPACE IF EXISTS chat;
CREATE NAMESPACE chat;

-- Conversations: top-level chat threads, listed in the sidebar.
CREATE TABLE chat.conversations (
  id          TEXT PRIMARY KEY,
  title       TEXT NOT NULL,
  created_at  TIMESTAMP NOT NULL,
  updated_at  TIMESTAMP NOT NULL
);

-- Messages: human and assistant turns, plus tool-call markers.
CREATE TABLE chat.messages (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL,
  role             TEXT NOT NULL,         -- 'user' | 'assistant' | 'system'
  body             TEXT NOT NULL,
  status           TEXT NOT NULL,         -- 'pending' | 'streaming' | 'final' | 'cancelled' | 'error'
  created_at       TIMESTAMP NOT NULL,
  updated_at       TIMESTAMP NOT NULL
);

-- Typing tokens: streamed deltas while the agent is generating.
-- Cleared when the message reaches status='final' or 'cancelled'.
CREATE TABLE chat.typing_tokens (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL,
  message_id       TEXT NOT NULL,
  body             TEXT NOT NULL,
  seq              INTEGER NOT NULL,
  created_at       TIMESTAMP NOT NULL
);

-- Approvals: human-in-the-loop checkpoints the agent waits on.
CREATE TABLE chat.approvals (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL,
  message_id       TEXT NOT NULL,
  question         TEXT NOT NULL,
  status           TEXT NOT NULL,         -- 'pending' | 'approved' | 'rejected'
  created_at       TIMESTAMP NOT NULL,
  resolved_at      TIMESTAMP
);

-- Tasks: in-flight agent work. The agent watches its own task row and bails
-- when is_cancelled flips to true (this is the live-query-as-control-plane
-- pattern — a Stop button is just an UPDATE on this row).
-- TYPE = 'USER' is required for ON INSERT-sourced topics to carry user
-- metadata; runConsumer rejects events without it.
CREATE TABLE chat.tasks (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL,
  message_id       TEXT NOT NULL,
  is_cancelled     BOOLEAN NOT NULL,
  started_at       TIMESTAMP NOT NULL,
  finished_at      TIMESTAMP
) WITH (TYPE = 'USER');

-- Topic: every new task row is published as a message on chat.task_events.
-- The agent subscribes via runConsumer (Kafka-style work queue with at-least-
-- once delivery + group-based load balancing). The chat.tasks row stays as
-- the cancellation channel — the agent watches it via a live query and
-- aborts when is_cancelled flips to true.
CREATE TOPIC chat.task_events;
ALTER TOPIC chat.task_events ADD SOURCE chat.tasks ON INSERT;
