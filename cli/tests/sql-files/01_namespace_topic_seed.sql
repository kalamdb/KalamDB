-- Namespace-local table and topic bootstrap with seed data.

DROP NAMESPACE IF EXISTS sql_file_case_01 CASCADE;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_01;
USE sql_file_case_01;

CREATE TABLE IF NOT EXISTS case01_conversations (
  id BIGINT PRIMARY KEY,
  title TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active',
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

CREATE TABLE IF NOT EXISTS case01_messages (
  id BIGINT PRIMARY KEY,
  conversation_id BIGINT NOT NULL,
  role TEXT NOT NULL,
  content TEXT NOT NULL,
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

DELETE FROM case01_messages WHERE id IN (2001, 2002);
DELETE FROM case01_conversations WHERE id IN (1001);

CREATE TOPIC IF NOT EXISTS message_events;
ALTER TOPIC message_events ADD SOURCE case01_messages ON INSERT WITH (payload = 'full');

INSERT INTO case01_conversations (id, title) VALUES (1001, 'Seeded namespace conversation');
INSERT INTO case01_messages (id, conversation_id, role, content)
VALUES
  (2001, 1001, 'user', 'hello from case 01'),
  (2002, 1001, 'assistant', 'reply from case 01');

CONSUME FROM message_events LIMIT 10;

DROP TOPIC message_events;
DROP NAMESPACE IF EXISTS sql_file_case_01 CASCADE;