-- Mix additional supported dialect statements with DML lifecycle.

DROP NAMESPACE IF EXISTS sql_file_case_19 CASCADE;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_19;
USE sql_file_case_19;

DROP TABLE IF EXISTS case19_metrics;

CREATE TABLE IF NOT EXISTS case19_metrics (
  id BIGINT PRIMARY KEY,
  metric_name TEXT NOT NULL,
  metric_value DOUBLE NOT NULL,
  is_valid BOOLEAN NOT NULL DEFAULT true,
  source_user TEXT NOT NULL DEFAULT CURRENT_USER(),
  created_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

DELETE FROM case19_metrics WHERE id IN (1901, 1902);
INSERT INTO case19_metrics (id, metric_name, metric_value)
VALUES (1901, 'cpu', 0.82), (1902, 'mem', 0.67);

CREATE TOPIC IF NOT EXISTS case19_probe_topic;
CLEAR TOPIC case19_probe_topic;
DROP TOPIC case19_probe_topic;

UPDATE case19_metrics SET metric_value = metric_value + 0.05 WHERE id = 1901;
DELETE FROM case19_metrics WHERE id = 1902;
INSERT INTO case19_metrics (id, metric_name, metric_value, is_valid)
VALUES (1902, 'mem-reinsert', 0.71, true);

DROP NAMESPACE IF EXISTS sql_file_case_19 CASCADE;