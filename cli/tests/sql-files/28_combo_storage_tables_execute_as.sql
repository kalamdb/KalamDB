-- Comprehensive combo coverage: storage lifecycle, all table types, wide table, batch DML,
-- transactions, namespace switching, multi-source topics, and EXECUTE AS user.

DROP USER IF EXISTS 'case28_user';
DROP NAMESPACE IF EXISTS sql_file_case_28_a CASCADE;
DROP NAMESPACE IF EXISTS sql_file_case_28_b CASCADE;
DROP STORAGE IF EXISTS case28_storage;

CREATE STORAGE case28_storage
  TYPE 'filesystem'
  PATH './data/storage_case28';

ALTER STORAGE case28_storage
  SET DESCRIPTION 'case28 storage';

CREATE USER 'case28_user'
  WITH PASSWORD 'Case28Pass!123'
  ROLE user
  EMAIL 'case28_user@example.com'
  STORAGE_MODE table
  STORAGE_ID 'case28_storage';

ALTER USER 'case28_user' SET EMAIL 'case28_user_new@example.com';

CREATE NAMESPACE IF NOT EXISTS sql_file_case_28_a;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_28_b;

USE sql_file_case_28_a;

CREATE TABLE IF NOT EXISTS case28_wide_user (
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
  c16_state TEXT NOT NULL DEFAULT 'new',
  c17_score BIGINT NOT NULL DEFAULT 0,
  c18_note TEXT
) WITH (
  TYPE = 'USER',
  STORAGE_ID = 'case28_storage',
  USE_USER_STORAGE = true,
  FLUSH_POLICY = 'rows:40,interval:20',
  COMPRESSION = 'snappy'
);

CREATE SHARED TABLE IF NOT EXISTS case28_shared_events (
  id BIGINT PRIMARY KEY,
  ref_id BIGINT NOT NULL,
  state TEXT NOT NULL,
  amount BIGINT NOT NULL,
  updated_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (
  STORAGE_ID = 'case28_storage',
  ACCESS_LEVEL = 'PUBLIC',
  FLUSH_POLICY = 'rows:40,interval:20',
  COMPRESSION = 'zstd'
);

CREATE TABLE IF NOT EXISTS case28_stream_events (
  id BIGINT PRIMARY KEY,
  src TEXT NOT NULL,
  payload TEXT NOT NULL,
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (
  TYPE = 'STREAM',
  TTL_SECONDS = 3600,
  EVICTION_STRATEGY = 'hybrid',
  MAX_STREAM_SIZE_BYTES = 2097152
);

USE sql_file_case_28_b;
CREATE TABLE IF NOT EXISTS case28_aux (
  id BIGINT PRIMARY KEY,
  label TEXT NOT NULL,
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:20,interval:10');

USE sql_file_case_28_a;

EXECUTE AS 'case28_user' (
  DELETE FROM case28_wide_user WHERE id IN (2801,2802,2803,2804,2805,2806,2807,2808,2809,2810)
);
DELETE FROM case28_shared_events WHERE id IN (2890, 2891, 2892);

CREATE TOPIC IF NOT EXISTS case28_topic;
ALTER TOPIC case28_topic ADD SOURCE case28_wide_user ON INSERT;
ALTER TOPIC case28_topic ADD SOURCE case28_shared_events ON INSERT WITH (payload = 'full');
ALTER TOPIC case28_topic ADD SOURCE case28_shared_events ON UPDATE WHERE state = 'done';

EXECUTE AS 'case28_user' (
  INSERT INTO case28_wide_user (
    id, tenant_id, c01_text, c02_text, c03_int, c04_int, c05_big, c06_big,
    c07_double, c08_double, c09_float, c10_float, c11_bool, c12_bool,
    c16_state, c17_score, c18_note
  ) VALUES
    (2801,1,'a1','b1',1,10,100,1000,1.1,2.1,3.1,4.1,true,false,'new',10,'row-1'),
    (2802,1,'a2','b2',2,20,200,2000,1.2,2.2,3.2,4.2,false,true,'new',20,'row-2'),
    (2803,1,'a3','b3',3,30,300,3000,1.3,2.3,3.3,4.3,true,false,'new',30,'row-3'),
    (2804,1,'a4','b4',4,40,400,4000,1.4,2.4,3.4,4.4,false,true,'new',40,'row-4'),
    (2805,1,'a5','b5',5,50,500,5000,1.5,2.5,3.5,4.5,true,false,'new',50,'row-5'),
    (2806,1,'a6','b6',6,60,600,6000,1.6,2.6,3.6,4.6,false,true,'new',60,'row-6'),
    (2807,1,'a7','b7',7,70,700,7000,1.7,2.7,3.7,4.7,true,false,'new',70,'row-7'),
    (2808,1,'a8','b8',8,80,800,8000,1.8,2.8,3.8,4.8,false,true,'new',80,'row-8'),
    (2809,1,'a9','b9',9,90,900,9000,1.9,2.9,3.9,4.9,true,false,'new',90,'row-9'),
    (2810,1,'a10','b10',10,100,1000,10000,2.0,3.0,4.0,5.0,false,true,'new',100,'row-10')
);

  STORAGE FLUSH TABLE sql_file_case_28_a.case28_wide_user;

BEGIN;
INSERT INTO case28_shared_events (id, ref_id, state, amount) VALUES (2890, 2801, 'new', 5000);
UPDATE case28_shared_events SET state = 'done', updated_at = NOW() WHERE id = 2890;
DELETE FROM case28_shared_events WHERE id = 2890;
COMMIT;

EXECUTE AS 'case28_user' (
  UPDATE case28_wide_user SET c16_state = 'done', c13_ts = NOW() WHERE id IN (2801,2802,2803,2804,2805)
);

STORAGE FLUSH TABLE sql_file_case_28_a.case28_wide_user;

EXECUTE AS 'case28_user' (
  DELETE FROM case28_wide_user WHERE id IN (2809,2810)
);

EXECUTE AS 'case28_user' (
  INSERT INTO case28_wide_user (
    id, tenant_id, c01_text, c02_text, c03_int, c04_int, c05_big, c06_big,
    c07_double, c08_double, c09_float, c10_float, c11_bool, c12_bool,
    c16_state, c17_score, c18_note
  ) VALUES
    (2809,1,'a9r','b9r',9,90,900,9000,1.95,2.95,3.95,4.95,true,false,'done',95,'row-9-r'),
    (2810,1,'a10r','b10r',10,100,1000,10000,2.05,3.05,4.05,5.05,false,true,'done',105,'row-10-r')
);

  STORAGE FLUSH TABLE sql_file_case_28_a.case28_wide_user;

INSERT INTO case28_stream_events (id, src, payload)
VALUES (2881, 'case28', 'stream-a'), (2882, 'case28', 'stream-b');

ALTER TABLE case28_wide_user ADD COLUMN IF NOT EXISTS extra_a TEXT DEFAULT 'na';
ALTER TABLE case28_wide_user ADD COLUMN IF NOT EXISTS extra_b BIGINT DEFAULT 0;

ALTER TABLE case28_wide_user
SET TBLPROPERTIES (
  STORAGE_ID = 'local',
  USE_USER_STORAGE = false
);

ALTER TABLE case28_shared_events
SET TBLPROPERTIES (STORAGE_ID = 'local');

ALTER USER 'case28_user' SET STORAGE_ID NULL;

CONSUME FROM case28_topic LIMIT 50;

DROP TOPIC case28_topic;
DROP NAMESPACE IF EXISTS sql_file_case_28_b CASCADE;
DROP NAMESPACE IF EXISTS sql_file_case_28_a CASCADE;
DROP USER IF EXISTS 'case28_user';
DROP STORAGE case28_storage;
