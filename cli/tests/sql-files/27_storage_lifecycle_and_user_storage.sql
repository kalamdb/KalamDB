-- Storage lifecycle coverage + user storage assignment + default topic payload route syntax.

DROP USER IF EXISTS 'case27_user';
DROP NAMESPACE IF EXISTS sql_file_case_27 CASCADE;
DROP STORAGE IF EXISTS case27_storage;

CREATE STORAGE case27_storage
  TYPE 'filesystem'
  PATH './data/storage_case27';

ALTER STORAGE case27_storage
  SET DESCRIPTION 'case27 storage';

ALTER STORAGE case27_storage
  SET NAME 'Case 27 Storage';

SHOW STORAGES;

CREATE USER 'case27_user'
  WITH PASSWORD 'Case27Pass!123'
  ROLE user
  EMAIL 'case27_user@example.com'
  STORAGE_MODE table
  STORAGE_ID 'case27_storage';

ALTER USER 'case27_user' SET STORAGE_MODE table;
ALTER USER 'case27_user' SET STORAGE_ID 'case27_storage';

CREATE NAMESPACE IF NOT EXISTS sql_file_case_27;
USE sql_file_case_27;

CREATE TABLE IF NOT EXISTS case27_orders (
  id BIGINT PRIMARY KEY,
  status TEXT NOT NULL,
  total BIGINT NOT NULL,
  updated_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (
  TYPE = 'USER',
  STORAGE_ID = 'case27_storage',
  USE_USER_STORAGE = true,
  FLUSH_POLICY = 'rows:50,interval:30',
  COMPRESSION = 'snappy'
);

DELETE FROM case27_orders WHERE id IN (2701, 2702);

EXECUTE AS 'case27_user' (
  DELETE FROM case27_orders WHERE id IN (2701, 2702)
);

CREATE TOPIC IF NOT EXISTS case27_order_activity;
ALTER TOPIC case27_order_activity ADD SOURCE case27_orders ON UPDATE WHERE status = 'done';

EXECUTE AS 'case27_user' (
  INSERT INTO case27_orders (id, status, total)
  VALUES (2701, 'new', 4200)
);

EXECUTE AS 'case27_user' (
  UPDATE case27_orders
  SET status = 'done', updated_at = NOW()
  WHERE id = 2701
);

INSERT INTO case27_orders (id, status, total)
VALUES (2702, 'new', 8000);

UPDATE case27_orders
SET status = 'done', updated_at = NOW()
WHERE id = 2702;

CONSUME FROM case27_order_activity LIMIT 20;

ALTER TABLE case27_orders
SET TBLPROPERTIES (
  STORAGE_ID = 'local',
  USE_USER_STORAGE = false
);

ALTER USER 'case27_user' SET STORAGE_ID NULL;

DROP TOPIC case27_order_activity;
DROP NAMESPACE IF EXISTS sql_file_case_27 CASCADE;
DROP USER IF EXISTS 'case27_user';
DROP STORAGE case27_storage;