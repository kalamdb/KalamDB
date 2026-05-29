-- PK delete/reinsert + select/update with NOW/CURRENT_USER and mixed datatypes.

DROP NAMESPACE IF EXISTS sql_file_case_16 CASCADE;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_16;
USE sql_file_case_16;

DROP TABLE IF EXISTS case16_events;

CREATE TABLE IF NOT EXISTS case16_events (
  id BIGINT PRIMARY KEY,
  title TEXT NOT NULL,
  qty INT NOT NULL,
  amount DOUBLE NOT NULL,
  ratio FLOAT,
  active BOOLEAN NOT NULL DEFAULT true,
  owner TEXT NOT NULL DEFAULT CURRENT_USER(),
  created_at TIMESTAMP NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

DELETE FROM case16_events WHERE id IN (1601, 1602, 1603);

INSERT INTO case16_events (id, title, qty, amount, ratio)
VALUES
  (1601, 'alpha', 3, 19.50, 0.5),
  (1602, 'beta', 7, 31.25, 0.7);

DELETE FROM case16_events WHERE id = 1601;
INSERT INTO case16_events (id, title, qty, amount, ratio)
VALUES (1601, 'alpha-reinsert', 9, 77.00, 0.9);

UPDATE case16_events
SET qty = qty + 1,
    amount = amount + 2.5,
    updated_at = NOW(),
    owner = CURRENT_USER()
WHERE id IN (1601, 1602);

DROP NAMESPACE IF EXISTS sql_file_case_16 CASCADE;