CREATE NAMESPACE IF NOT EXISTS okf_sync;

CREATE USER TABLE okf_sync.context_files (
  path TEXT PRIMARY KEY,
  file_ref FILE NOT NULL,
  created_at TIMESTAMP DEFAULT NOW(),
  updated_at TIMESTAMP DEFAULT NOW()
);
