-- Drop and recreate objects inside one namespace-scoped batch.

DROP NAMESPACE IF EXISTS sql_file_case_10 CASCADE;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_10;
USE sql_file_case_10;

CREATE TABLE IF NOT EXISTS draft_messages (
  id BIGINT PRIMARY KEY,
  body TEXT NOT NULL,
  status TEXT NOT NULL
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

CREATE TOPIC IF NOT EXISTS draft_events;
ALTER TOPIC draft_events ADD SOURCE draft_messages ON INSERT WITH (payload = 'full');
INSERT INTO draft_messages (id, body, status)
VALUES (10101, 'first draft', 'pending');
CONSUME FROM draft_events LIMIT 10;

DROP TOPIC draft_events;
DROP TABLE IF EXISTS draft_messages;

CREATE TABLE IF NOT EXISTS draft_messages (
  id BIGINT PRIMARY KEY,
  body TEXT NOT NULL,
  status TEXT NOT NULL
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

CREATE TOPIC IF NOT EXISTS draft_events_recreated;
ALTER TOPIC draft_events_recreated ADD SOURCE draft_messages ON INSERT WITH (payload = 'full');
INSERT INTO draft_messages (id, body, status)
VALUES (10102, 'second draft', 'ready');
CONSUME FROM draft_events_recreated LIMIT 10;

DROP TOPIC draft_events_recreated;
DROP NAMESPACE IF EXISTS sql_file_case_10 CASCADE;