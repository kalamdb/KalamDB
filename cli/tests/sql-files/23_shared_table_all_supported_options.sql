-- SHARED table: access level + storage/flush/compression + DML and topic routing.

DROP NAMESPACE IF EXISTS sql_file_case_23 CASCADE;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_23;
USE sql_file_case_23;

DROP TABLE IF EXISTS case23_shared_feed;
CREATE TABLE IF NOT EXISTS case23_shared_feed (
  id BIGINT PRIMARY KEY,
  actor TEXT NOT NULL,
  event TEXT NOT NULL,
  updated_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (
  TYPE = 'SHARED',
  ACCESS_LEVEL = 'PUBLIC',
  STORAGE_ID = 'local',
  FLUSH_POLICY = 'rows:90,interval:40',
  COMPRESSION = 'zstd'
);

DELETE FROM case23_shared_feed WHERE id IN (2301, 2302);

CREATE TOPIC IF NOT EXISTS case23_shared_topic;
ALTER TOPIC case23_shared_topic ADD SOURCE case23_shared_feed ON INSERT WITH (payload = 'full');
ALTER TOPIC case23_shared_topic ADD SOURCE case23_shared_feed ON UPDATE WITH (payload = 'full');
ALTER TOPIC case23_shared_topic ADD SOURCE case23_shared_feed ON DELETE WITH (payload = 'full');

INSERT INTO case23_shared_feed (id, actor, event)
VALUES (2301, 'admin', 'seed'), (2302, 'system', 'boot');

UPDATE case23_shared_feed
SET event = 'seed-updated', updated_at = NOW()
WHERE id = 2301;

DELETE FROM case23_shared_feed WHERE id = 2302;
INSERT INTO case23_shared_feed (id, actor, event)
VALUES (2302, 'system', 'reinserted');

ALTER TABLE case23_shared_feed ADD COLUMN scope TEXT;
ALTER TABLE case23_shared_feed ALTER COLUMN scope SET DEFAULT 'global';
ALTER TABLE case23_shared_feed ALTER COLUMN scope DROP DEFAULT;
ALTER TABLE case23_shared_feed DROP COLUMN scope;

ALTER TABLE case23_shared_feed SET ACCESS LEVEL RESTRICTED;
ALTER TABLE case23_shared_feed SET TBLPROPERTIES (
  ACCESS_LEVEL = 'PRIVATE',
  STORAGE_ID = 'local',
  FLUSH_POLICY = 'rows:30,interval:15',
  COMPRESSION = 'snappy'
);

CONSUME FROM case23_shared_topic LIMIT 40;

DROP TOPIC case23_shared_topic;
DROP NAMESPACE IF EXISTS sql_file_case_23 CASCADE;