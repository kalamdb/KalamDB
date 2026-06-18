-- Topic retention set and clear flow inside a namespace-scoped batch.

DROP NAMESPACE IF EXISTS sql_file_case_02 CASCADE;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_02;
USE sql_file_case_02;

CREATE TABLE IF NOT EXISTS audit_log (
  id BIGINT PRIMARY KEY,
  level TEXT NOT NULL,
  body TEXT NOT NULL,
  archived_at TIMESTAMP,
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

CREATE TOPIC IF NOT EXISTS audit_topic;
ALTER TOPIC audit_topic ADD SOURCE audit_log ON INSERT WITH (payload = 'full');
ALTER TOPIC audit_topic ADD SOURCE audit_log ON UPDATE WHERE archived_at IS NOT NULL WITH (payload = 'full');
ALTER TOPIC audit_topic ADD SOURCE audit_log ON DELETE WITH (payload = 'full');

INSERT INTO audit_log (id, level, body)
VALUES (2101, 'info', 'retention configured for audit topic');

UPDATE audit_log SET archived_at = NOW() WHERE id = 2101;
DELETE FROM audit_log WHERE id = 2101;

CONSUME FROM audit_topic LIMIT 5;

DROP TOPIC audit_topic;
DROP NAMESPACE IF EXISTS sql_file_case_02 CASCADE;