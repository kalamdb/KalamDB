-- Exercise USE + SET NAMESPACE and ensure unqualified DML follows active namespace.

DROP NAMESPACE IF EXISTS sql_file_case_18_a CASCADE;
DROP NAMESPACE IF EXISTS sql_file_case_18_b CASCADE;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_18_a;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_18_b;

USE sql_file_case_18_a;
DROP TABLE IF EXISTS case18_items;
CREATE TABLE IF NOT EXISTS case18_items (
  id BIGINT PRIMARY KEY,
  label TEXT NOT NULL,
  updated_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');
DELETE FROM case18_items WHERE id IN (1801, 1803);
INSERT INTO case18_items (id, label) VALUES (1801, 'from-a');

USE sql_file_case_18_b;
DROP TABLE IF EXISTS case18_items;
CREATE TABLE IF NOT EXISTS case18_items (
  id BIGINT PRIMARY KEY,
  label TEXT NOT NULL,
  updated_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');
DELETE FROM case18_items WHERE id IN (1802, 1804);
INSERT INTO case18_items (id, label) VALUES (1802, 'from-b');

USE sql_file_case_18_a;
INSERT INTO case18_items (id, label) VALUES (1803, 'from-a-again');
UPDATE case18_items SET label = 'from-a-updated', updated_at = NOW() WHERE id = 1803;

USE sql_file_case_18_b;
INSERT INTO case18_items (id, label) VALUES (1804, 'from-b-again');
DELETE FROM case18_items WHERE id = 1802;
INSERT INTO case18_items (id, label) VALUES (1802, 'from-b-reinsert');

DROP NAMESPACE IF EXISTS sql_file_case_18_b CASCADE;
DROP NAMESPACE IF EXISTS sql_file_case_18_a CASCADE;