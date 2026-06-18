-- Delete and reinsert the same primary keys with topic fan-out on DML operations.

DROP NAMESPACE IF EXISTS sql_file_case_14 CASCADE;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_14;
USE sql_file_case_14;

CREATE TABLE IF NOT EXISTS ledger (
  id BIGINT PRIMARY KEY,
  category TEXT NOT NULL,
  amount BIGINT NOT NULL,
  note TEXT,
  updated_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

DELETE FROM ledger WHERE id IN (1401, 1402);

CREATE TOPIC IF NOT EXISTS ledger_mutations;
ALTER TOPIC ledger_mutations ADD SOURCE ledger ON INSERT WITH (payload = 'full');
ALTER TOPIC ledger_mutations ADD SOURCE ledger ON UPDATE WITH (payload = 'full');
ALTER TOPIC ledger_mutations ADD SOURCE ledger ON DELETE WITH (payload = 'full');

INSERT INTO ledger (id, category, amount, note)
VALUES (1401, 'income', 500, 'first insert'), (1402, 'expense', 120, 'initial expense');

DELETE FROM ledger WHERE id = 1401;
INSERT INTO ledger (id, category, amount, note) VALUES (1401, 'income', 650, 'reinsert after delete');

UPDATE ledger SET amount = 90, note = 'reduced expense', updated_at = NOW() WHERE id = 1402;
DELETE FROM ledger WHERE id = 1402;
INSERT INTO ledger (id, category, amount, note) VALUES (1402, 'expense', 80, 'reinsert second key');

CONSUME FROM ledger_mutations LIMIT 40;

DROP TOPIC ledger_mutations;
DROP NAMESPACE IF EXISTS sql_file_case_14 CASCADE;