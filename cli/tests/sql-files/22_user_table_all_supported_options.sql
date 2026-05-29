-- USER table: create with supported options + alter table properties + DML lifecycle.

DROP NAMESPACE IF EXISTS sql_file_case_22 CASCADE;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_22;
USE sql_file_case_22;

DROP TABLE IF EXISTS case22_user_orders;
CREATE TABLE IF NOT EXISTS case22_user_orders (
  id BIGINT PRIMARY KEY,
  buyer TEXT NOT NULL,
  amount BIGINT NOT NULL,
  state TEXT NOT NULL DEFAULT 'new',
  updated_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (
  TYPE = 'USER',
  STORAGE_ID = 'local',
  USE_USER_STORAGE = true,
  FLUSH_POLICY = 'rows:120,interval:45',
  COMPRESSION = 'zstd'
);

DELETE FROM case22_user_orders WHERE id IN (2201, 2202);

INSERT INTO case22_user_orders (id, buyer, amount)
VALUES (2201, 'alice', 50), (2202, 'bob', 80);

UPDATE case22_user_orders
SET amount = amount + 10,
    state = 'updated',
    updated_at = NOW()
WHERE id = 2201;

DELETE FROM case22_user_orders WHERE id = 2202;
INSERT INTO case22_user_orders (id, buyer, amount, state)
VALUES (2202, 'bob', 95, 'reinserted');

ALTER TABLE case22_user_orders ADD COLUMN note TEXT;
ALTER TABLE case22_user_orders MODIFY COLUMN amount BIGINT;
ALTER TABLE case22_user_orders ALTER COLUMN note SET DEFAULT 'none';
ALTER TABLE case22_user_orders ALTER COLUMN note DROP DEFAULT;
ALTER TABLE case22_user_orders DROP COLUMN note;

ALTER TABLE case22_user_orders SET TBLPROPERTIES (
  STORAGE_ID = 'local',
  USE_USER_STORAGE = false,
  FLUSH_POLICY = 'rows:60,interval:20',
  COMPRESSION = 'snappy'
);

DROP NAMESPACE IF EXISTS sql_file_case_22 CASCADE;