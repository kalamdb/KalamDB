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
