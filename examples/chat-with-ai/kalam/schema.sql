-- Chat With AI
--
-- Schema-first KalamDB app: one SQL file, generated client types, and
-- TypeScript procedures deployed with `kalam deploy` / `kalam dev`.
-- There is no separate agent process.
--
-- How a sent message travels:
--   1. The browser calls chat_demo.send_message (room or personal inbox).
--   2. That insert fans out on topic chat_demo.ai_inbox.
--   3. Trigger chat_demo.process_user_message runs chat_demo.on_user_message.
--   4. The procedure writes STREAM thinking/typing rows, then the assistant reply.
--   5. Every member tab sees both steps through live queries.
--
-- Table kinds used here:
--   SHARED  one copy of the data, then CREATE POLICY decides who sees which rows
--   USER    per-user personal inbox (direct_messages)
--   STREAM  ephemeral progress rows (thinking / typing) with a short TTL
--   TOPIC   fan-out of INSERTs so a procedure can react without polling

CREATE NAMESPACE IF NOT EXISTS chat_demo;

-- This file is the source of truth for `kalam dev`. Dropping first lets the
-- same script recreate a clean local demo.
DROP TRIGGER IF EXISTS chat_demo.process_user_message;
DROP PROCEDURE IF EXISTS chat_demo.on_user_message;
DROP PROCEDURE IF EXISTS chat_demo.send_message;
DROP PROCEDURE IF EXISTS chat_demo.join_room;
DROP TYPE IF EXISTS chat_demo.message_target;
DROP TOPIC IF EXISTS chat_demo.ai_inbox;
DROP TABLE IF EXISTS chat_demo.agent_events;
DROP TABLE IF EXISTS chat_demo.messages;
DROP TABLE IF EXISTS chat_demo.direct_messages;
DROP TABLE IF EXISTS chat_demo.room_members;
DROP TABLE IF EXISTS chat_demo.rooms;

-- Rooms everyone can list and create. Membership still gates the transcript.
CREATE SHARED TABLE IF NOT EXISTS chat_demo.rooms (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Membership. One TEXT primary key: KalamDB does not support composite PKs yet.
-- id is '{user_id}:{room_id}' so a user can join a room at most once.
CREATE SHARED TABLE IF NOT EXISTS chat_demo.room_members (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    room_id TEXT NOT NULL
);

-- Durable room transcript. Policies keep each user inside rooms they joined.
CREATE SHARED TABLE IF NOT EXISTS chat_demo.messages (
    id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
    room TEXT NOT NULL DEFAULT 'main',
    role TEXT NOT NULL,
    author TEXT NOT NULL,
    sender_username TEXT NOT NULL,
    content TEXT NOT NULL,
    reply_to BIGINT,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Personal copilot inbox. Each signed-in user has their own copy of this table.
CREATE USER TABLE IF NOT EXISTS chat_demo.direct_messages (
    id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
    role TEXT NOT NULL,
    author TEXT NOT NULL,
    sender_username TEXT NOT NULL,
    content TEXT NOT NULL,
    reply_to BIGINT,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_messages_room ON chat_demo.messages (room);
CREATE INDEX IF NOT EXISTS idx_messages_reply_to ON chat_demo.messages (reply_to);
CREATE INDEX IF NOT EXISTS idx_direct_messages_reply_to ON chat_demo.direct_messages (reply_to);
CREATE INDEX IF NOT EXISTS idx_room_members_user ON chat_demo.room_members (user_id);

-- Live "the copilot is thinking / typing" rows. STREAM + TTL so they fade away.
-- `room` is the destination id (room id or username). `scope` is 'room' | 'direct'.
CREATE STREAM TABLE IF NOT EXISTS chat_demo.agent_events (
    id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
    response_id TEXT NOT NULL,
    room TEXT NOT NULL DEFAULT 'main',
    scope TEXT NOT NULL DEFAULT 'room',
    sender_username TEXT NOT NULL,
    stage TEXT NOT NULL,
    preview TEXT NOT NULL DEFAULT '',
    message TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TTL_SECONDS = 10);

-- Room directory is public so join_room (INVOKER) can find an existing room
-- before inserting membership. Messages stay membership-gated below.
CREATE POLICY rooms_visible ON chat_demo.rooms
  FOR SELECT TO user
  USING (true);

-- Anyone signed in can create a room. Joining is a separate membership insert.
CREATE POLICY rooms_create ON chat_demo.rooms
  FOR INSERT TO user
  WITH CHECK (true);

-- You can add or remove only your own membership rows.
CREATE POLICY room_members_self ON chat_demo.room_members
  FOR ALL TO user
  USING (user_id = CURRENT_USER)
  WITH CHECK (user_id = CURRENT_USER);

-- Messages in a room are visible only to members of that room.
CREATE POLICY messages_member_select ON chat_demo.messages
  FOR SELECT TO user
  USING (
    room IN (
      SELECT room_id FROM chat_demo.room_members
      WHERE user_id = CURRENT_USER
    )
  );

-- You can only post into a room you belong to.
CREATE POLICY messages_member_insert ON chat_demo.messages
  FOR INSERT TO user
  WITH CHECK (
    room IN (
      SELECT room_id FROM chat_demo.room_members
      WHERE user_id = CURRENT_USER
    )
  );

-- Same membership rule for edits.
CREATE POLICY messages_member_update ON chat_demo.messages
  FOR UPDATE TO user
  USING (
    room IN (
      SELECT room_id FROM chat_demo.room_members
      WHERE user_id = CURRENT_USER
    )
  )
  WITH CHECK (
    room IN (
      SELECT room_id FROM chat_demo.room_members
      WHERE user_id = CURRENT_USER
    )
  );

CREATE TYPE chat_demo.message_target AS ENUM ('room', 'direct');

-- Topic payload type is implicit: a tagged union of these source row types,
-- discriminated by `_table` (`chat_demo:messages` | `chat_demo:direct_messages`).
CREATE TOPIC IF NOT EXISTS chat_demo.ai_inbox;
ALTER TOPIC chat_demo.ai_inbox ADD SOURCE chat_demo.messages ON INSERT;
ALTER TOPIC chat_demo.ai_inbox ADD SOURCE chat_demo.direct_messages ON INSERT;

-- Join (or create) a room. Browser clients call this instead of inserting membership.
CREATE PROCEDURE chat_demo.join_room(room_id TEXT NOT NULL)
RETURNS TEXT
SECURITY INVOKER;

-- Post a user message into a room transcript or the caller's personal inbox.
-- Returns the inserted row tagged with its source table, same shape as PAYLOAD.
CREATE PROCEDURE chat_demo.send_message(
    target chat_demo.message_target NOT NULL,
    target_id TEXT NOT NULL,
    content TEXT NOT NULL
)
RETURNS chat_demo.ai_inbox
SECURITY INVOKER;

-- Topic-trigger handler. PAYLOAD is chat_demo.ai_inbox (union of source rows).
CREATE PROCEDURE chat_demo.on_user_message(payload chat_demo.ai_inbox NOT NULL)
SECURITY DEFINER;

GRANT EXECUTE ON PROCEDURE chat_demo.join_room TO user;
GRANT EXECUTE ON PROCEDURE chat_demo.send_message TO user;

-- Seed before the trigger so these INSERTs are not handled as live user mail.
INSERT INTO chat_demo.rooms (id, title)
VALUES ('main', 'Main');

INSERT INTO chat_demo.room_members (id, user_id, room_id)
VALUES ('root:main', 'root', 'main'), ('admin:main', 'admin', 'main');

INSERT INTO chat_demo.messages (role, author, sender_username, content)
VALUES ('user', 'user_1', 'root', 'Hello everyone!');

INSERT INTO chat_demo.messages (role, author, sender_username, content)
VALUES ('assistant', 'ai_bot', 'assistant', 'Hi, how can I help?');

-- STREAM progress is per chatting user (EXECUTE AS). All members see the
-- committed SHARED assistant row through live queries.
CREATE TRIGGER chat_demo.process_user_message
  ON TOPIC chat_demo.ai_inbox
  EXECUTE PROCEDURE chat_demo.on_user_message(PAYLOAD)
  WITH (
    principal = 'system',
    start = 'latest',
    retries = 5,
    retry_backoff = '1s',
    concurrency = 1
  );
