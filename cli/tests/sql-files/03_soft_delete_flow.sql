-- Soft-delete topic routing with delete and update filters.

DROP NAMESPACE IF EXISTS sql_file_case_03 CASCADE;
CREATE NAMESPACE IF NOT EXISTS sql_file_case_03;
USE sql_file_case_03;

CREATE TABLE IF NOT EXISTS projects (
  id BIGINT PRIMARY KEY,
  name TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'active',
  deleted_at TIMESTAMP,
  updated_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

CREATE TOPIC IF NOT EXISTS project_cleanup;
ALTER TOPIC project_cleanup ADD SOURCE projects ON DELETE WITH (payload = 'full');
ALTER TOPIC project_cleanup ADD SOURCE projects ON UPDATE WHERE deleted_at IS NOT NULL WITH (payload = 'full');

INSERT INTO projects (id, name) VALUES (3101, 'legacy-project');
UPDATE projects SET state = 'deleted', deleted_at = NOW(), updated_at = NOW() WHERE id = 3101;
DELETE FROM projects WHERE id = 3101;

CONSUME FROM project_cleanup LIMIT 10;

DROP TOPIC project_cleanup;
DROP NAMESPACE IF EXISTS sql_file_case_03 CASCADE;