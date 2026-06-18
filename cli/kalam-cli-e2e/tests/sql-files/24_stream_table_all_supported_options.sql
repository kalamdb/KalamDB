-- STREAM table: required TTL and stream-only options + alter table stream properties.

DROP NAMESPACE IF EXISTS sql_file_case_24 CASCADE;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_24;
USE sql_file_case_24;

DROP TABLE IF EXISTS case24_stream_events;
CREATE TABLE IF NOT EXISTS case24_stream_events (
  id BIGINT PRIMARY KEY,
  source TEXT NOT NULL,
  payload TEXT NOT NULL,
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (
  TYPE = 'STREAM',
  TTL_SECONDS = 3600,
  EVICTION_STRATEGY = 'hybrid',
  MAX_STREAM_SIZE_BYTES = 1048576
);

CREATE TOPIC IF NOT EXISTS case24_stream_topic;
ALTER TOPIC case24_stream_topic ADD SOURCE case24_stream_events ON INSERT WITH (payload = 'full');

INSERT INTO case24_stream_events (id, source, payload)
VALUES
  (2401, 'sensor-a', 'temp=21'),
  (2402, 'sensor-b', 'temp=24');

INSERT INTO case24_stream_events (id, source, payload)
VALUES (2403, 'sensor-c', 'temp=19');

ALTER TABLE case24_stream_events ADD COLUMN partition_key TEXT;
ALTER TABLE case24_stream_events ALTER COLUMN partition_key SET DEFAULT 'p0';
ALTER TABLE case24_stream_events ALTER COLUMN partition_key DROP DEFAULT;
ALTER TABLE case24_stream_events DROP COLUMN partition_key;

ALTER TABLE case24_stream_events SET TBLPROPERTIES (
  TTL_SECONDS = 5400,
  EVICTION_STRATEGY = 'size_based',
  MAX_STREAM_SIZE_BYTES = 2097152
);

CONSUME FROM case24_stream_topic LIMIT 40;

DROP TOPIC case24_stream_topic;
DROP NAMESPACE IF EXISTS sql_file_case_24 CASCADE;