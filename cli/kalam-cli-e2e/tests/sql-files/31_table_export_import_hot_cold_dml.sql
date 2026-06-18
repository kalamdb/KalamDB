-- Comprehensive table export/import lifecycle:
--   Part A: SHARED table - hot + cold rows, export, import, DML on imported copy.
--   Part B: USER table - two users each with their own data scope; per-user export and
--           import into a shared target table so both scopes land in the same destination.

DROP NAMESPACE IF EXISTS sql_file_case_31 CASCADE;
DROP USER IF EXISTS 'case31_user_alice';
DROP USER IF EXISTS 'case31_user_bob';

CREATE USER 'case31_user_alice' WITH PASSWORD 'Case31Alice!123' ROLE user;
CREATE USER 'case31_user_bob' WITH PASSWORD 'Case31Bob!123' ROLE user;

CREATE NAMESPACE IF NOT EXISTS sql_file_case_31;
USE NAMESPACE sql_file_case_31;

-- Part A: SHARED table

CREATE TABLE IF NOT EXISTS case31_shared (
  id BIGINT PRIMARY KEY,
  label TEXT NOT NULL,
  amount BIGINT NOT NULL
) WITH (
  TYPE = 'SHARED',
  STORAGE_ID = 'local',
  FLUSH_POLICY = 'rows:2,interval:3600'
);

-- First two inserts hit the flush threshold and may land in cold (Parquet) storage.
INSERT INTO case31_shared (id, label, amount)
VALUES (3101, 'cold-a', 100), (3102, 'cold-b', 200);

-- These writes remain hot so the export covers both storage tiers.
UPDATE case31_shared SET amount = 250 WHERE id = 3102;
INSERT INTO case31_shared (id, label, amount) VALUES (3103, 'hot-c', 300);

-- Export the shared table (export executor triggers flush + waits before archiving).
export sql_file_case_31.case31_shared --output /tmp/kalamdb-case31-shared.zip;

-- Create the import target before running import.
CREATE TABLE IF NOT EXISTS case31_shared_imported (
  id BIGINT PRIMARY KEY,
  label TEXT NOT NULL,
  amount BIGINT NOT NULL
) WITH (
  TYPE = 'SHARED',
  STORAGE_ID = 'local',
  FLUSH_POLICY = 'rows:100,interval:3600'
);

import sql_file_case_31.case31_shared_imported /tmp/kalamdb-case31-shared.zip;

-- DML on the imported shared data to prove the copy is independently writable.
UPDATE case31_shared_imported SET label = 'import-updated' WHERE id = 3101;
INSERT INTO case31_shared_imported (id, label, amount) VALUES (3199, 'import-new', 999);
DELETE FROM case31_shared_imported WHERE id = 3102;

SELECT id, label, amount FROM case31_shared_imported ORDER BY id;

-- Part B: USER table - two users, per-user export/import

CREATE TABLE IF NOT EXISTS case31_user_data (
  id BIGINT PRIMARY KEY,
  owner TEXT NOT NULL,
  note TEXT NOT NULL,
  score BIGINT NOT NULL DEFAULT 0
) WITH (
  TYPE = 'USER',
  STORAGE_ID = 'local',
  FLUSH_POLICY = 'rows:2,interval:3600'
);

-- Seed rows for alice (first two trigger flush threshold for her scope).
EXECUTE AS USER 'case31_user_alice' (
  INSERT INTO case31_user_data (id, owner, note, score)
  VALUES (3201, 'alice', 'cold-row-1', 10), (3202, 'alice', 'cold-row-2', 20)
);
EXECUTE AS USER 'case31_user_alice' (
  INSERT INTO case31_user_data (id, owner, note, score)
  VALUES (3203, 'alice', 'hot-row-3', 30)
);
EXECUTE AS USER 'case31_user_alice' (
  UPDATE case31_user_data SET score = 99 WHERE id = 3202
);

-- Seed rows for bob (same structure, independent user scope).
EXECUTE AS USER 'case31_user_bob' (
  INSERT INTO case31_user_data (id, owner, note, score)
  VALUES (3211, 'bob', 'cold-row-1', 110), (3212, 'bob', 'cold-row-2', 120)
);
EXECUTE AS USER 'case31_user_bob' (
  INSERT INTO case31_user_data (id, owner, note, score)
  VALUES (3213, 'bob', 'hot-row-3', 130)
);

-- Create the shared import target for both user scopes before any import runs.
CREATE TABLE IF NOT EXISTS case31_user_imported (
  id BIGINT PRIMARY KEY,
  owner TEXT NOT NULL,
  note TEXT NOT NULL,
  score BIGINT NOT NULL DEFAULT 0
) WITH (
  TYPE = 'USER',
  STORAGE_ID = 'local',
  FLUSH_POLICY = 'rows:100,interval:3600'
);

-- Export alice's scope, then import it into the target.
export sql_file_case_31.case31_user_data --user-id case31_user_alice --output /tmp/kalamdb-case31-alice.zip;
import sql_file_case_31.case31_user_imported /tmp/kalamdb-case31-alice.zip --user-id case31_user_alice;

-- Export bob's scope, then import it into the same target table.
export sql_file_case_31.case31_user_data --user-id case31_user_bob --output /tmp/kalamdb-case31-bob.zip;
import sql_file_case_31.case31_user_imported /tmp/kalamdb-case31-bob.zip --user-id case31_user_bob;

-- DML on the imported user data for both scopes.
EXECUTE AS USER 'case31_user_alice' (
  UPDATE case31_user_imported SET note = 'alice-import-updated' WHERE id = 3201
);
EXECUTE AS USER 'case31_user_alice' (
  INSERT INTO case31_user_imported (id, owner, note, score) VALUES (3299, 'alice', 'alice-new', 999)
);
EXECUTE AS USER 'case31_user_bob' (
  DELETE FROM case31_user_imported WHERE id = 3211
);
EXECUTE AS USER 'case31_user_bob' (
  UPDATE case31_user_imported SET note = 'bob-import-updated' WHERE id = 3212
);

-- Verify both user scopes are readable after DML.
EXECUTE AS USER 'case31_user_alice' (
  SELECT id, owner, note, score FROM case31_user_imported ORDER BY id
);
EXECUTE AS USER 'case31_user_bob' (
  SELECT id, owner, note, score FROM case31_user_imported ORDER BY id
);

-- Cleanup

DROP NAMESPACE IF EXISTS sql_file_case_31 CASCADE;
DROP USER IF EXISTS 'case31_user_alice';
DROP USER IF EXISTS 'case31_user_bob';
