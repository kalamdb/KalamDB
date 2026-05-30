-- Chat agent setup fixture used by CLI batch-file admin tests.

CREATE NAMESPACE IF NOT EXISTS chat;
USE chat;

DROP TABLE IF EXISTS messages;
DROP TABLE IF EXISTS conversations;

CREATE TABLE IF NOT EXISTS conversations (
  id BIGINT PRIMARY KEY,
  title TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active',
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (
  TYPE = 'SHARED',
  STORAGE_ID = 'local'
);

CREATE TABLE IF NOT EXISTS messages (
  id BIGINT PRIMARY KEY,
  conversation_id BIGINT NOT NULL,
  role TEXT NOT NULL,
  content TEXT NOT NULL,
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (
  TYPE = 'SHARED',
  STORAGE_ID = 'local'
);

CREATE TOPIC IF NOT EXISTS message_sources_topic;
ALTER TOPIC message_sources_topic ADD SOURCE messages ON INSERT WITH (payload = 'full');

CREATE TOPIC IF NOT EXISTS conversation_delete_sources_topic;
ALTER TOPIC conversation_delete_sources_topic ADD SOURCE conversations ON UPDATE WITH (payload = 'full');
ALTER TOPIC conversation_delete_sources_topic ADD SOURCE conversations ON DELETE WITH (payload = 'full');

INSERT INTO conversations (id, title, status)
VALUES (1, 'Chat with AI Agent', 'active');

INSERT INTO messages (id, conversation_id, role, content)
VALUES
  (1, 1, 'user', 'Hello there'),
  (2, 1, 'assistant', 'Hi, how can I help?');

UPDATE conversations SET status = 'deleted' WHERE id = 1;
