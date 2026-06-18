-- Multiple topics in one namespace sharing source tables.

DROP NAMESPACE IF EXISTS sql_file_case_09 CASCADE;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_09;
USE sql_file_case_09;

CREATE TABLE IF NOT EXISTS sessions (
  id BIGINT PRIMARY KEY,
  state TEXT NOT NULL,
  expired_at TIMESTAMP
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

CREATE TABLE IF NOT EXISTS session_metrics (
  id BIGINT PRIMARY KEY,
  session_id BIGINT NOT NULL,
  metric TEXT NOT NULL,
  metric_value BIGINT NOT NULL
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

DELETE FROM session_metrics WHERE id IN (9201);
DELETE FROM sessions WHERE id IN (9101);

CREATE TOPIC IF NOT EXISTS session_updates;
CREATE TOPIC IF NOT EXISTS session_expiry;

ALTER TOPIC session_updates ADD SOURCE sessions ON INSERT WITH (payload = 'full');
ALTER TOPIC session_updates ADD SOURCE session_metrics ON INSERT WITH (payload = 'full');
ALTER TOPIC session_expiry ADD SOURCE sessions ON UPDATE WHERE expired_at IS NOT NULL WITH (payload = 'full');

INSERT INTO sessions (id, state) VALUES (9101, 'open');
INSERT INTO session_metrics (id, session_id, metric, metric_value)
VALUES (9201, 9101, 'latency_ms', 18);
UPDATE sessions SET state = 'expired', expired_at = NOW() WHERE id = 9101;

CONSUME FROM session_updates LIMIT 20;
CONSUME FROM session_expiry LIMIT 20;

DROP TOPIC session_expiry;
DROP TOPIC session_updates;
DROP NAMESPACE IF EXISTS sql_file_case_09 CASCADE;