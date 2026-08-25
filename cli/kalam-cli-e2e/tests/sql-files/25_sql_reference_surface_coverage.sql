-- Coverage for documented SQL reference surface that was not previously exercised.

DROP NAMESPACE IF EXISTS sql_file_case_25 CASCADE;
DROP STORAGE IF EXISTS case25_storage;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_25;
ALTER NAMESPACE sql_file_case_25 SET DESCRIPTION 'case 25 namespace description';

SHOW NAMESPACES;

USE NAMESPACE sql_file_case_25;
SET NAMESPACE sql_file_case_25;

CREATE TABLE IF NOT EXISTS case25_user_tbl (
  id BIGINT PRIMARY KEY,
  msg TEXT NOT NULL,
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (
  TYPE = 'USER',
  STORAGE_ID = 'local',
  FLUSH_POLICY = 'rows:50,interval:30',
  COMPRESSION = 'snappy'
);

CREATE SHARED TABLE IF NOT EXISTS case25_shared_tbl (
  id BIGINT PRIMARY KEY,
  val TEXT NOT NULL,
  updated_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (
  COMPRESSION = 'zstd'
);

CREATE STREAM TABLE IF NOT EXISTS case25_stream_tbl (
  id BIGINT PRIMARY KEY,
  payload TEXT NOT NULL,
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (
  TTL_SECONDS = 120,
  EVICTION_STRATEGY = 'hybrid',
  MAX_STREAM_SIZE_BYTES = 131072
);

SHOW TABLES;

DESCRIBE TABLE case25_user_tbl;
DESC TABLE case25_user_tbl;
SHOW STATS FOR TABLE case25_user_tbl;

DELETE FROM case25_user_tbl WHERE id IN (2501, 2502, 2503);
DELETE FROM case25_shared_tbl WHERE id IN (2511);

BEGIN;
INSERT INTO case25_user_tbl (id, msg) VALUES (2501, 'tx begin commit');
UPDATE case25_user_tbl SET msg = 'tx committed' WHERE id = 2501;
COMMIT WORK;

START TRANSACTION;
INSERT INTO case25_user_tbl (id, msg) VALUES (2502, 'tx rollback');
ROLLBACK;

BEGIN;
INSERT INTO case25_shared_tbl (id, val) VALUES (2511, 'shared tx commit');
COMMIT;

CREATE POLICY case25_shared_select ON sql_file_case_25.case25_shared_tbl
  FOR SELECT TO PUBLIC USING (true);
DROP POLICY case25_shared_select ON sql_file_case_25.case25_shared_tbl;

CREATE TOPIC sql_file_case_25.case25_topic
PARTITIONS 1
WITH (retention_seconds = 1800, retention_max_bytes = 1048576);

ALTER TOPIC sql_file_case_25.case25_topic ADD SOURCE case25_user_tbl ON INSERT WITH (payload = 'full');
ALTER TOPIC sql_file_case_25.case25_topic ADD SOURCE case25_user_tbl ON UPDATE WITH (payload = 'full');
ALTER TOPIC sql_file_case_25.case25_topic ADD SOURCE case25_user_tbl ON DELETE WITH (payload = 'full');

INSERT INTO case25_user_tbl (id, msg) VALUES (2503, 'topic insert event');
UPDATE case25_user_tbl SET msg = 'topic update event' WHERE id = 2503;
DELETE FROM case25_user_tbl WHERE id = 2503;

CONSUME FROM sql_file_case_25.case25_topic
GROUP 'case25-group'
FROM EARLIEST
LIMIT 20;

ACK sql_file_case_25.case25_topic
GROUP 'case25-group'
PARTITION 0
UPTO OFFSET 0;

RESET CONSUMER GROUP 'case25-group'
ON sql_file_case_25.case25_topic
PARTITION 0
TO 0;

CLEAR TOPIC sql_file_case_25.case25_topic;
DROP TOPIC sql_file_case_25.case25_topic;

CREATE STORAGE case25_storage
  TYPE 'filesystem'
  PATH './data/storage_case25';

ALTER STORAGE case25_storage
  SET DESCRIPTION 'case 25 storage';

SHOW STORAGES;
STORAGE CHECK case25_storage;
SHOW MANIFEST;

-- Additional management surface coverage
STORAGE FLUSH ALL IN sql_file_case_25;
STORAGE COMPACT TABLE sql_file_case_25.case25_user_tbl;
STORAGE COMPACT ALL IN sql_file_case_25;
EXPORT USER DATA;
SHOW EXPORT;

DROP STORAGE case25_storage;
DROP NAMESPACE IF EXISTS sql_file_case_25 CASCADE;