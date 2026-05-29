-- Exercise clear topic and reseed data in the active namespace.

DROP NAMESPACE IF EXISTS sql_file_case_08 CASCADE;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_08;
USE sql_file_case_08;

CREATE TABLE IF NOT EXISTS sensor_events (
  id BIGINT PRIMARY KEY,
  sensor_id TEXT NOT NULL,
  reading BIGINT NOT NULL,
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

DELETE FROM sensor_events WHERE sensor_id IN ('sensor-a', 'sensor-b', 'sensor-c');

CREATE TOPIC IF NOT EXISTS sensor_topic;
ALTER TOPIC sensor_topic ADD SOURCE sensor_events ON INSERT WITH (payload = 'full');

INSERT INTO sensor_events (id, sensor_id, reading)
VALUES (8101, 'sensor-a', 55), (8102, 'sensor-a', 61);

CONSUME FROM sensor_topic LIMIT 10;
CLEAR TOPIC sensor_topic;

INSERT INTO sensor_events (id, sensor_id, reading)
VALUES (8103, 'sensor-b', 44), (8104, 'sensor-c', 72);

CONSUME FROM sensor_topic LIMIT 10;

DROP TOPIC sensor_topic;
DROP NAMESPACE IF EXISTS sql_file_case_08 CASCADE;