-- Add/drop/modify column operations with PK delete/reinsert checks.

DROP NAMESPACE IF EXISTS sql_file_case_20 CASCADE;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_20;
USE sql_file_case_20;

DROP TABLE IF EXISTS case20_orders;

CREATE TABLE IF NOT EXISTS case20_orders (
  id BIGINT PRIMARY KEY,
  code TEXT NOT NULL,
  quantity INT NOT NULL,
  status TEXT NOT NULL DEFAULT 'new',
  updated_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

DELETE FROM case20_orders WHERE code IN ('A-001', 'A-002', 'A-002-R', 'A-003');

ALTER TABLE case20_orders ADD COLUMN discount DOUBLE DEFAULT 0.0;

INSERT INTO case20_orders (id, code, quantity)
VALUES (2001, 'A-001', 5), (2002, 'A-002', 2);
ALTER TABLE case20_orders MODIFY COLUMN quantity BIGINT;

UPDATE case20_orders
SET quantity = quantity + 10,
    discount = 1.5,
    updated_at = NOW(),
    status = 'processed'
WHERE code = 'A-001';

DELETE FROM case20_orders WHERE code = 'A-002';
INSERT INTO case20_orders (id, code, quantity, status)
VALUES (2002, 'A-002-R', 8, 'reinserted');

ALTER TABLE case20_orders DROP COLUMN discount;

INSERT INTO case20_orders (id, code, quantity, status)
VALUES (2003, 'A-003', 1, 'manual');

DROP NAMESPACE IF EXISTS sql_file_case_20 CASCADE;