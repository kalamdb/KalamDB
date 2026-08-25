-- Combo coverage with namespace drop/recreate cycle, wide table, batch writes,
-- all table types, storage lifecycle, user execute-as, and multiple topic sources.

DROP USER IF EXISTS 'case30_user';
DROP NAMESPACE IF EXISTS sql_file_case_30_main CASCADE;
DROP NAMESPACE IF EXISTS sql_file_case_30_other CASCADE;
DROP STORAGE IF EXISTS case30_storage;

CREATE STORAGE case30_storage
  TYPE 'filesystem'
  PATH './data/storage_case30';

ALTER STORAGE case30_storage SET DESCRIPTION 'case30 storage';

CREATE USER 'case30_user'
  WITH PASSWORD 'Case30Pass!123'
  ROLE user
  EMAIL 'case30_user@example.com'
  STORAGE_MODE table
  STORAGE_ID 'case30_storage';

ALTER USER 'case30_user' SET ROLE service;
ALTER USER 'case30_user' SET ROLE user;

DROP NAMESPACE IF EXISTS sql_file_case_30_other CASCADE;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_30_main;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_30_other;

USE sql_file_case_30_main;

CREATE TABLE IF NOT EXISTS case30_wide_user (
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
  c16_state TEXT NOT NULL DEFAULT 'draft',
  c17_score BIGINT NOT NULL DEFAULT 0,
  c18_note TEXT
) WITH (
  TYPE = 'USER',
  STORAGE_ID = 'case30_storage',
  USE_USER_STORAGE = true,
  FLUSH_POLICY = 'rows:25,interval:12',
  COMPRESSION = 'snappy'
);

CREATE SHARED TABLE IF NOT EXISTS case30_shared (
  id BIGINT PRIMARY KEY,
  kind TEXT NOT NULL,
  value BIGINT NOT NULL,
  state TEXT NOT NULL,
  updated_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (
  STORAGE_ID = 'case30_storage',
  FLUSH_POLICY = 'rows:25,interval:12',
  COMPRESSION = 'zstd'
);

CREATE TABLE IF NOT EXISTS case30_stream (
  id BIGINT PRIMARY KEY,
  src TEXT NOT NULL,
  payload TEXT NOT NULL,
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (
  TYPE = 'STREAM',
  TTL_SECONDS = 2400,
  EVICTION_STRATEGY = 'hybrid',
  MAX_STREAM_SIZE_BYTES = 1048576
);

USE sql_file_case_30_other;
CREATE TABLE IF NOT EXISTS case30_other_table (
  id BIGINT PRIMARY KEY,
  label TEXT NOT NULL
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:10,interval:5');

DROP NAMESPACE IF EXISTS sql_file_case_30_other CASCADE;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_30_other;
USE sql_file_case_30_other;
CREATE TABLE IF NOT EXISTS case30_other_table2 (
  id BIGINT PRIMARY KEY,
  label TEXT NOT NULL,
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TYPE = 'SHARED');

USE sql_file_case_30_main;

EXECUTE AS 'case30_user' (
  DELETE FROM case30_wide_user WHERE id IN (3001,3002,3003,3004,3005,3006,3007,3008,3009,3010)
);
DELETE FROM case30_shared WHERE id IN (3091, 3092, 3093);

CREATE TOPIC IF NOT EXISTS case30_topic;
ALTER TOPIC case30_topic ADD SOURCE case30_wide_user ON INSERT;
ALTER TOPIC case30_topic ADD SOURCE case30_wide_user ON UPDATE WHERE c16_state = 'published';
ALTER TOPIC case30_topic ADD SOURCE case30_shared ON INSERT;
ALTER TOPIC case30_topic ADD SOURCE case30_shared ON UPDATE WHERE state = 'ready';

EXECUTE AS 'case30_user' (
  INSERT INTO case30_wide_user (
    id, tenant_id, c01_text, c02_text, c03_int, c04_int, c05_big, c06_big,
    c07_double, c08_double, c09_float, c10_float, c11_bool, c12_bool,
    c16_state, c17_score, c18_note
  ) VALUES
    (3001,3,'m1','n1',1,12,120,1200,1.12,2.12,3.12,4.12,true,false,'draft',12,'c1'),
    (3002,3,'m2','n2',2,24,240,2400,1.24,2.24,3.24,4.24,false,true,'draft',24,'c2'),
    (3003,3,'m3','n3',3,36,360,3600,1.36,2.36,3.36,4.36,true,false,'draft',36,'c3'),
    (3004,3,'m4','n4',4,48,480,4800,1.48,2.48,3.48,4.48,false,true,'draft',48,'c4'),
    (3005,3,'m5','n5',5,60,600,6000,1.60,2.60,3.60,4.60,true,false,'draft',60,'c5'),
    (3006,3,'m6','n6',6,72,720,7200,1.72,2.72,3.72,4.72,false,true,'draft',72,'c6'),
    (3007,3,'m7','n7',7,84,840,8400,1.84,2.84,3.84,4.84,true,false,'draft',84,'c7'),
    (3008,3,'m8','n8',8,96,960,9600,1.96,2.96,3.96,4.96,false,true,'draft',96,'c8'),
    (3009,3,'m9','n9',9,108,1080,10800,2.08,3.08,4.08,5.08,true,false,'draft',108,'c9'),
    (3010,3,'m10','n10',10,120,1200,12000,2.20,3.20,4.20,5.20,false,true,'draft',120,'c10')
);

  STORAGE FLUSH TABLE sql_file_case_30_main.case30_wide_user;

BEGIN;
INSERT INTO case30_shared (id, kind, value, state) VALUES (3091, 'alpha', 500, 'new');
UPDATE case30_shared SET state = 'ready', updated_at = NOW() WHERE id = 3091;
DELETE FROM case30_shared WHERE id = 3091;
COMMIT;

EXECUTE AS 'case30_user' (
  UPDATE case30_wide_user SET c16_state = 'published', c13_ts = NOW() WHERE id IN (3001,3002,3003,3004)
);

STORAGE FLUSH TABLE sql_file_case_30_main.case30_wide_user;

INSERT INTO case30_stream (id, src, payload)
VALUES (3081, 'case30', 's1'), (3082, 'case30', 's2');

ALTER TABLE case30_wide_user ADD COLUMN IF NOT EXISTS extra_a TEXT DEFAULT 'z';
ALTER TABLE case30_wide_user ADD COLUMN IF NOT EXISTS extra_b BIGINT DEFAULT 1;

ALTER TABLE case30_wide_user
SET TBLPROPERTIES (
  STORAGE_ID = 'local',
  USE_USER_STORAGE = false
);

ALTER TABLE case30_shared
SET TBLPROPERTIES (STORAGE_ID = 'local');

ALTER USER 'case30_user' SET STORAGE_ID NULL;

CONSUME FROM case30_topic LIMIT 50;

DROP TOPIC case30_topic;
DROP NAMESPACE IF EXISTS sql_file_case_30_other CASCADE;
DROP NAMESPACE IF EXISTS sql_file_case_30_main CASCADE;
DROP USER IF EXISTS 'case30_user';
DROP STORAGE case30_storage;
