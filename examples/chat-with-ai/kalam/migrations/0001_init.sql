-- Migration: init
-- Created: 2026-06-27T00:00:00Z
-- Read kalam/schema.sql for the commented walkthrough of this demo.

-- UP
CREATE NAMESPACE IF NOT EXISTS chat_demo;

DROP TABLE IF EXISTS chat_demo.agent_events;
DROP TABLE IF EXISTS chat_demo.messages;
DROP TABLE IF EXISTS chat_demo.room_members;
DROP TABLE IF EXISTS chat_demo.rooms;

CREATE SHARED TABLE IF NOT EXISTS chat_demo.rooms (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE SHARED TABLE IF NOT EXISTS chat_demo.room_members (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    room_id TEXT NOT NULL
);

CREATE SHARED TABLE IF NOT EXISTS chat_demo.messages (
    id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
    room TEXT NOT NULL DEFAULT 'main',
    role TEXT NOT NULL,
    author TEXT NOT NULL,
    sender_username TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE STREAM TABLE IF NOT EXISTS chat_demo.agent_events (
    id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
    response_id TEXT NOT NULL,
    room TEXT NOT NULL DEFAULT 'main',
    sender_username TEXT NOT NULL,
    stage TEXT NOT NULL,
    preview TEXT NOT NULL DEFAULT '',
    message TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TTL_SECONDS = 10);

CREATE POLICY rooms_member_select ON chat_demo.rooms
  FOR SELECT TO user
  USING (
    id IN (
      SELECT room_id FROM chat_demo.room_members
      WHERE user_id = CURRENT_USER
    )
  );

CREATE POLICY rooms_create ON chat_demo.rooms
  FOR INSERT TO user
  WITH CHECK (true);

CREATE POLICY room_members_self ON chat_demo.room_members
  FOR ALL TO user
  USING (user_id = CURRENT_USER)
  WITH CHECK (user_id = CURRENT_USER);

CREATE POLICY messages_member_select ON chat_demo.messages
  FOR SELECT TO user
  USING (
    room IN (
      SELECT room_id FROM chat_demo.room_members
      WHERE user_id = CURRENT_USER
    )
  );

CREATE POLICY messages_member_insert ON chat_demo.messages
  FOR INSERT TO user
  WITH CHECK (
    room IN (
      SELECT room_id FROM chat_demo.room_members
      WHERE user_id = CURRENT_USER
    )
  );

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

CREATE TOPIC IF NOT EXISTS chat_demo.ai_inbox;
ALTER TOPIC chat_demo.ai_inbox ADD SOURCE chat_demo.messages ON INSERT;

INSERT INTO chat_demo.rooms (id, title)
VALUES ('main', 'Main');

INSERT INTO chat_demo.room_members (id, user_id, room_id)
VALUES ('root:main', 'root', 'main'), ('admin:main', 'admin', 'main');

INSERT INTO chat_demo.messages (role, author, sender_username, content)
VALUES ('user', 'user_1', 'root', 'Hello everyone!');

INSERT INTO chat_demo.messages (role, author, sender_username, content)
VALUES ('assistant', 'ai_bot', 'assistant', 'Hi, how can I help?');

-- DOWN
DROP TOPIC chat_demo.ai_inbox;
DROP TABLE IF EXISTS chat_demo.agent_events;
DROP TABLE IF EXISTS chat_demo.messages;
DROP TABLE IF EXISTS chat_demo.room_members;
DROP TABLE IF EXISTS chat_demo.rooms;
