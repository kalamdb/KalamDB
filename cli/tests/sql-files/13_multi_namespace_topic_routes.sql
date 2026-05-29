-- Use two namespaces and route events from both explicit and current namespace references.

DROP NAMESPACE IF EXISTS sql_file_case_13_a CASCADE;
DROP NAMESPACE IF EXISTS sql_file_case_13_b CASCADE;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_13_a;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_13_b;

USE sql_file_case_13_a;
CREATE TABLE IF NOT EXISTS tasks (
  id BIGINT PRIMARY KEY,
  title TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'new'
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');
DELETE FROM tasks WHERE id IN (1301, 1303);
INSERT INTO tasks (id, title) VALUES (1301, 'task-a1');

USE sql_file_case_13_b;
CREATE TABLE IF NOT EXISTS tasks (
  id BIGINT PRIMARY KEY,
  title TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'new'
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');
DELETE FROM tasks WHERE id IN (1302);

CREATE TOPIC IF NOT EXISTS b_task_events;
ALTER TOPIC b_task_events ADD SOURCE tasks ON INSERT WITH (payload = 'full');
ALTER TOPIC b_task_events ADD SOURCE tasks ON UPDATE WITH (payload = 'full');

INSERT INTO tasks (id, title) VALUES (1302, 'task-b1');
USE sql_file_case_13_a;
INSERT INTO tasks (id, title) VALUES (1303, 'task-a2-after-switch');
USE sql_file_case_13_b;
UPDATE tasks SET status = 'done' WHERE id = 1302;

CONSUME FROM b_task_events LIMIT 30;

DROP TOPIC b_task_events;
DROP NAMESPACE IF EXISTS sql_file_case_13_b CASCADE;
DROP NAMESPACE IF EXISTS sql_file_case_13_a CASCADE;