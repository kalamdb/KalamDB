CREATE TABLE inventory (
  id TEXT NOT NULL,
  sku TEXT PRIMARY KEY,
  tags TEXT[],
  attachment FILE
);
