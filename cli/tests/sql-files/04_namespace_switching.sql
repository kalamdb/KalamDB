-- Multiple USE statements in one file to exercise per-statement namespace carry-over.

DROP NAMESPACE IF EXISTS sql_file_case_04_a CASCADE;
DROP NAMESPACE IF EXISTS sql_file_case_04_b CASCADE;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_04_a;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_04_b;

USE sql_file_case_04_a;
CREATE TABLE IF NOT EXISTS sql_file_case_04_a.items (
  id BIGINT PRIMARY KEY,
  name TEXT NOT NULL
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');
INSERT INTO sql_file_case_04_a.items (id, name) VALUES (4101, 'from-a');

USE sql_file_case_04_b;
CREATE TABLE IF NOT EXISTS sql_file_case_04_b.items (
  id BIGINT PRIMARY KEY,
  name TEXT NOT NULL
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');
INSERT INTO sql_file_case_04_b.items (id, name) VALUES (4201, 'from-b');

CREATE TOPIC IF NOT EXISTS item_events;
ALTER TOPIC item_events ADD SOURCE sql_file_case_04_b.items ON INSERT WITH (payload = 'full');
INSERT INTO sql_file_case_04_b.items (id, name) VALUES (4202, 'second-b');
CONSUME FROM item_events LIMIT 10;

DROP TOPIC item_events;
DROP NAMESPACE IF EXISTS sql_file_case_04_b CASCADE;
DROP NAMESPACE IF EXISTS sql_file_case_04_a CASCADE;