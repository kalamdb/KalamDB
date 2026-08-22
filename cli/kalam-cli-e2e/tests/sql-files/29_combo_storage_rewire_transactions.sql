-- Combo coverage focused on storage rewiring, transactions, wide schema, and multi-source topics.

DROP USER IF EXISTS 'case29_user';
DROP NAMESPACE IF EXISTS sql_file_case_29_a CASCADE;
DROP NAMESPACE IF EXISTS sql_file_case_29_b CASCADE;
DROP STORAGE IF EXISTS case29_storage_b;
DROP STORAGE IF EXISTS case29_storage_a;

CREATE STORAGE case29_storage_a
  TYPE 'filesystem'
  PATH './data/storage_case29_a';

CREATE STORAGE case29_storage_b
  TYPE 'filesystem'
  PATH './data/storage_case29_b';

ALTER STORAGE case29_storage_b
  SET DESCRIPTION 'case29 storage b';

CREATE USER 'case29_user'
  WITH PASSWORD 'Case29Pass!123'
  ROLE user
  EMAIL 'case29_user@example.com'
  STORAGE_MODE table
  STORAGE_ID 'case29_storage_a';

ALTER USER 'case29_user' SET STORAGE_ID 'case29_storage_b';

CREATE NAMESPACE IF NOT EXISTS sql_file_case_29_a;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_29_b;

USE sql_file_case_29_a;

CREATE TABLE IF NOT EXISTS case29_wide_user (
  id BIGINT PRIMARY KEY,
  tenant_id BIGINT NOT NULL,
  c01_text TEXT NOT NULL,
  c02_text TEXT NOT NULL,
  c03_int INTEGER NOT NULL,
  c04_int INTEGER NOT NULL,
  c05_big BIGINT NOT NULL,
  c06_big BIGINT NOT NULL,
  c07_double DOUBLE NOT NULL,
  c08_double DOUBLE NOT NULL,
  c09_float FLOAT NOT NULL,
  c10_float FLOAT NOT NULL,
  c11_bool BOOLEAN NOT NULL DEFAULT true,
  c12_bool BOOLEAN NOT NULL DEFAULT false,
  c13_ts TIMESTAMP NOT NULL DEFAULT NOW(),
  c14_ts TIMESTAMP NOT NULL DEFAULT NOW(),
  c15_actor TEXT NOT NULL DEFAULT CURRENT_USER(),
  c16_state TEXT NOT NULL DEFAULT 'open',
  c17_score BIGINT NOT NULL DEFAULT 0,
  c18_note TEXT
) WITH (
  TYPE = 'USER',
  STORAGE_ID = 'case29_storage_a',
  USE_USER_STORAGE = true,
  FLUSH_POLICY = 'rows:30,interval:15',
  COMPRESSION = 'snappy'
);

CREATE SHARED TABLE IF NOT EXISTS case29_shared_orders (
  id BIGINT PRIMARY KEY,
  label TEXT NOT NULL,
  amount BIGINT NOT NULL,
  status TEXT NOT NULL,
  updated_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (
  STORAGE_ID = 'case29_storage_a',
  FLUSH_POLICY = 'rows:30,interval:15',
  COMPRESSION = 'zstd'
);

CREATE TABLE IF NOT EXISTS case29_stream_log (
  id BIGINT PRIMARY KEY,
  source TEXT NOT NULL,
  payload TEXT NOT NULL,
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (
  TYPE = 'STREAM',
  TTL_SECONDS = 1800,
  EVICTION_STRATEGY = 'size_based',
  MAX_STREAM_SIZE_BYTES = 1048576
);

USE sql_file_case_29_b;
CREATE TABLE IF NOT EXISTS case29_other (
  id BIGINT PRIMARY KEY,
  body TEXT NOT NULL
) WITH (TYPE = 'SHARED');
USE sql_file_case_29_a;

EXECUTE AS 'case29_user' (
  DELETE FROM case29_wide_user WHERE id IN (2901,2902,2903,2904,2905,2906,2907,2908,2909,2910)
);
DELETE FROM case29_shared_orders WHERE id IN (2991, 2992, 2993);

CREATE TOPIC IF NOT EXISTS case29_topic;
ALTER TOPIC case29_topic ADD SOURCE case29_wide_user ON INSERT;
ALTER TOPIC case29_topic ADD SOURCE case29_wide_user ON UPDATE WHERE c16_state = 'closed';
ALTER TOPIC case29_topic ADD SOURCE case29_shared_orders ON INSERT;

EXECUTE AS 'case29_user' (
  INSERT INTO case29_wide_user (
    id, tenant_id, c01_text, c02_text, c03_int, c04_int, c05_big, c06_big,
    c07_double, c08_double, c09_float, c10_float, c11_bool, c12_bool,
    c16_state, c17_score, c18_note
  ) VALUES
    (2901,2,'u1','v1',1,11,101,1001,1.11,2.11,3.11,4.11,true,false,'open',11,'b1'),
    (2902,2,'u2','v2',2,22,202,2002,1.22,2.22,3.22,4.22,false,true,'open',22,'b2'),
    (2903,2,'u3','v3',3,33,303,3003,1.33,2.33,3.33,4.33,true,false,'open',33,'b3'),
    (2904,2,'u4','v4',4,44,404,4004,1.44,2.44,3.44,4.44,false,true,'open',44,'b4'),
    (2905,2,'u5','v5',5,55,505,5005,1.55,2.55,3.55,4.55,true,false,'open',55,'b5'),
    (2906,2,'u6','v6',6,66,606,6006,1.66,2.66,3.66,4.66,false,true,'open',66,'b6'),
    (2907,2,'u7','v7',7,77,707,7007,1.77,2.77,3.77,4.77,true,false,'open',77,'b7'),
    (2908,2,'u8','v8',8,88,808,8008,1.88,2.88,3.88,4.88,false,true,'open',88,'b8'),
    (2909,2,'u9','v9',9,99,909,9009,1.99,2.99,3.99,4.99,true,false,'open',99,'b9'),
    (2910,2,'u10','v10',10,110,1010,10010,2.10,3.10,4.10,5.10,false,true,'open',110,'b10')
);

  STORAGE FLUSH TABLE sql_file_case_29_a.case29_wide_user;

BEGIN;
INSERT INTO case29_shared_orders (id, label, amount, status)
VALUES (2991, 'tx-1', 1000, 'new'), (2992, 'tx-2', 2000, 'new');
UPDATE case29_shared_orders SET status = 'closed', updated_at = NOW() WHERE id = 2992;
DELETE FROM case29_shared_orders WHERE id = 2991;
COMMIT;

EXECUTE AS 'case29_user' (
  UPDATE case29_wide_user SET c16_state = 'closed', c13_ts = NOW() WHERE id IN (2901,2902,2903)
);

STORAGE FLUSH TABLE sql_file_case_29_a.case29_wide_user;

INSERT INTO case29_stream_log (id, source, payload)
VALUES (2981, 'case29', 'evt-a'), (2982, 'case29', 'evt-b');

ALTER TABLE case29_wide_user ADD COLUMN IF NOT EXISTS extra_a TEXT DEFAULT 'x';
ALTER TABLE case29_wide_user ADD COLUMN IF NOT EXISTS extra_b BIGINT DEFAULT 0;

ALTER TABLE case29_wide_user
SET TBLPROPERTIES (
  STORAGE_ID = 'case29_storage_b',
  USE_USER_STORAGE = true,
  FLUSH_POLICY = 'rows:20,interval:10'
);

ALTER TABLE case29_wide_user
SET TBLPROPERTIES (
  STORAGE_ID = 'local',
  USE_USER_STORAGE = false
);

ALTER TABLE case29_shared_orders
SET TBLPROPERTIES (STORAGE_ID = 'local');

ALTER USER 'case29_user' SET STORAGE_ID NULL;

CONSUME FROM case29_topic LIMIT 50;

DROP TOPIC case29_topic;
DROP NAMESPACE IF EXISTS sql_file_case_29_b CASCADE;
DROP NAMESPACE IF EXISTS sql_file_case_29_a CASCADE;
DROP USER IF EXISTS 'case29_user';
DROP STORAGE case29_storage_b;
DROP STORAGE case29_storage_a;
