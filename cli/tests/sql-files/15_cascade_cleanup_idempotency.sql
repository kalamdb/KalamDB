-- Two full lifecycle passes in one script with CASCADE drops between passes.

DROP NAMESPACE IF EXISTS sql_file_case_15 CASCADE;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_15;
USE sql_file_case_15;

CREATE TABLE IF NOT EXISTS parents (
  id BIGINT PRIMARY KEY,
  name TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'active'
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

CREATE TABLE IF NOT EXISTS children (
  id BIGINT PRIMARY KEY,
  parent_id BIGINT NOT NULL,
  body TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'active'
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

DELETE FROM children WHERE id IN (1511);
DELETE FROM parents WHERE id IN (1501);

INSERT INTO parents (id, name) VALUES (1501, 'p1');
INSERT INTO children (id, parent_id, body) VALUES (1511, 1501, 'c1');
UPDATE children SET state = 'archived' WHERE id = 1511;
DELETE FROM children WHERE id = 1511;
INSERT INTO children (id, parent_id, body, state) VALUES (1511, 1501, 'c1-reinsert', 'active');

DROP NAMESPACE IF EXISTS sql_file_case_15 CASCADE;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_15;
USE sql_file_case_15;

CREATE TABLE IF NOT EXISTS parents (
  id BIGINT PRIMARY KEY,
  name TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'active'
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

DELETE FROM parents WHERE id IN (1599);

INSERT INTO parents (id, name, state) VALUES (1599, 'p2', 'final-pass');
UPDATE parents SET state = 'done' WHERE id = 1599;
DELETE FROM parents WHERE id = 1599;

DROP NAMESPACE IF EXISTS sql_file_case_15 CASCADE;