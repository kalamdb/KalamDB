-- ALTER TABLE lifecycle: add/modify/alter/drop columns with DML around each step.

DROP NAMESPACE IF EXISTS sql_file_case_17 CASCADE;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_17;
USE sql_file_case_17;

DROP TABLE IF EXISTS case17_users;

CREATE TABLE IF NOT EXISTS case17_users (
  id BIGINT PRIMARY KEY,
  username TEXT NOT NULL,
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

DELETE FROM case17_users WHERE id IN (1701, 1702, 1703);

ALTER TABLE case17_users ADD COLUMN age INT DEFAULT 0;
ALTER TABLE case17_users ADD COLUMN score FLOAT;
ALTER TABLE case17_users ADD COLUMN note TEXT;

INSERT INTO case17_users (id, username, age, score, note)
VALUES
  (1701, 'jamal', 30, 7.5, 'first pass'),
  (1702, 'nora', 26, 9.1, 'second pass');

ALTER TABLE case17_users MODIFY COLUMN age BIGINT;

UPDATE case17_users
SET score = score + 0.4,
    note = 'updated',
    age = age + 1
WHERE id IN (1701, 1702);

INSERT INTO case17_users (id, username, age, note)
VALUES (1703, 'ravi', 40, 'default score');

ALTER TABLE case17_users DROP COLUMN note;

DROP NAMESPACE IF EXISTS sql_file_case_17 CASCADE;