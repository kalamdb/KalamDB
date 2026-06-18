-- Explicitly drop and recreate the same namespace in one script.

DROP NAMESPACE IF EXISTS sql_file_case_11 CASCADE;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_11;
USE sql_file_case_11;

CREATE TABLE IF NOT EXISTS accounts (
  id BIGINT PRIMARY KEY,
  owner TEXT NOT NULL,
  balance BIGINT NOT NULL,
  state TEXT NOT NULL DEFAULT 'open'
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

DELETE FROM accounts WHERE id IN (1101, 1102);

CREATE TOPIC IF NOT EXISTS account_events;
ALTER TOPIC account_events ADD SOURCE accounts ON INSERT WITH (payload = 'full');
ALTER TOPIC account_events ADD SOURCE accounts ON UPDATE WITH (payload = 'full');
ALTER TOPIC account_events ADD SOURCE accounts ON DELETE WITH (payload = 'full');

INSERT INTO accounts (id, owner, balance) VALUES (1101, 'alpha', 5000), (1102, 'beta', 7000);
UPDATE accounts SET balance = 6500 WHERE id = 1101;
DELETE FROM accounts WHERE id = 1102;
INSERT INTO accounts (id, owner, balance, state) VALUES (1102, 'beta', 7200, 'reopened');

CONSUME FROM account_events LIMIT 20;

DROP TOPIC account_events;
DROP NAMESPACE IF EXISTS sql_file_case_11 CASCADE;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_11;
USE sql_file_case_11;

CREATE TABLE IF NOT EXISTS accounts (
  id BIGINT PRIMARY KEY,
  owner TEXT NOT NULL,
  balance BIGINT NOT NULL,
  state TEXT NOT NULL DEFAULT 'open'
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

DELETE FROM accounts WHERE id IN (1199);

INSERT INTO accounts (id, owner, balance) VALUES (1199, 'gamma', 9000);
UPDATE accounts SET state = 'closed' WHERE id = 1199;
DELETE FROM accounts WHERE id = 1199;

DROP NAMESPACE IF EXISTS sql_file_case_11 CASCADE;