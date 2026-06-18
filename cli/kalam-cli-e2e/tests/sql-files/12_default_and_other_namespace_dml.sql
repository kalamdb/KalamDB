-- Namespace-local DML only in the active namespace.

DROP NAMESPACE IF EXISTS sql_file_case_12 CASCADE;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_12;
USE sql_file_case_12;

CREATE TABLE IF NOT EXISTS case12_items (
  id BIGINT PRIMARY KEY,
  label TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'new'
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

DELETE FROM case12_items WHERE id IN (1201, 1202);

INSERT INTO case12_items (id, label) VALUES (1201, 'from-current-namespace');
INSERT INTO case12_items (id, label) VALUES (1202, 'same-namespace-second-row');
UPDATE case12_items SET state = 'updated-current' WHERE id = 1201;
DELETE FROM case12_items WHERE id = 1202;
INSERT INTO case12_items (id, label, state) VALUES (1202, 'reinsert-current', 'reinserted');

DROP NAMESPACE IF EXISTS sql_file_case_12 CASCADE;