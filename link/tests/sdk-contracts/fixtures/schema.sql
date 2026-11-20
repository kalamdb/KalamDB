CREATE TYPE mood AS ENUM ('calm', 'busy');
CREATE TYPE address AS (city TEXT);
CREATE TABLE inventory (
  id TEXT NOT NULL,
  sku TEXT PRIMARY KEY,
  tags TEXT[],
  dates TIMESTAMP[],
  moods mood[],
  addresses address[],
  attachment FILE
);

-- One table of each kind, so the generated bindings are checked against all
-- three row models and not just the kindless default above.
CREATE USER TABLE profiles (
  id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
  email TEXT NOT NULL,
  born_on DATE,
  wakes_at TIME,
  avatar BYTES,
  seen_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE SHARED TABLE rooms (
  id BIGINT PRIMARY KEY,
  name TEXT NOT NULL,
  mood mood
);

CREATE STREAM TABLE events (
  seq BIGINT PRIMARY KEY,
  payload JSON,
  at TIMESTAMP NOT NULL DEFAULT NOW()
);
