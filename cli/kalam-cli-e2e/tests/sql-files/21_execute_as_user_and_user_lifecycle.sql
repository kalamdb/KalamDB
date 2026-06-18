-- Create users and validate EXECUTE AS USER DML on a USER table.

DROP NAMESPACE IF EXISTS sql_file_case_21 CASCADE;

DROP USER IF EXISTS 'sql_case21_user_a';
DROP USER IF EXISTS 'sql_case21_user_b';

CREATE USER 'sql_case21_user_a' WITH PASSWORD 'Case21PassA123!' ROLE user;
CREATE USER 'sql_case21_user_b' WITH PASSWORD 'Case21PassB123!' ROLE user;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_21;
USE sql_file_case_21;

DROP TABLE IF EXISTS case21_actions;
CREATE TABLE IF NOT EXISTS case21_actions (
  id BIGINT PRIMARY KEY,
  actor TEXT NOT NULL,
  action TEXT NOT NULL,
  updated_at TIMESTAMP NOT NULL DEFAULT NOW()
) WITH (TYPE = 'USER', STORAGE_ID = 'local', FLUSH_POLICY = 'rows:100,interval:60');

EXECUTE AS USER 'sql_case21_user_a' (
  DELETE FROM case21_actions WHERE id IN (2101, 2102)
);

EXECUTE AS USER 'sql_case21_user_a' (
  INSERT INTO case21_actions (id, actor, action)
  VALUES (2101, 'sql_case21_user_a', 'inserted-by-execute-as')
);

EXECUTE AS USER 'sql_case21_user_a' (
  UPDATE case21_actions
  SET action = 'updated-by-execute-as', updated_at = NOW()
  WHERE id = 2101
);

EXECUTE AS USER 'sql_case21_user_a' (
  DELETE FROM case21_actions WHERE id = 2101
);

EXECUTE AS USER 'sql_case21_user_b' (
  INSERT INTO case21_actions (id, actor, action)
  VALUES (2102, 'sql_case21_user_b', 'user-b-insert')
);

EXECUTE AS USER 'sql_case21_user_b' (
  UPDATE case21_actions
  SET action = 'user-b-update', updated_at = NOW()
  WHERE id = 2102
);

EXECUTE AS USER 'sql_case21_user_b' (
  DELETE FROM case21_actions WHERE id = 2102
);

DROP NAMESPACE IF EXISTS sql_file_case_21 CASCADE;
DROP USER IF EXISTS 'sql_case21_user_a';
DROP USER IF EXISTS 'sql_case21_user_b';