-- Migration: draft
-- Updated: 2026-06-25T18:08:40.350113+00:00

-- UP
-- Generated KalamDB schema evolution
-- Review before applying in production.

CREATE NAMESPACE IF NOT EXISTS okf_sync;

CREATE USER TABLE okf_sync.context_files (
  path TEXT PRIMARY KEY,
  file_ref FILE NOT NULL,
  created_at TIMESTAMP DEFAULT NOW(),
  updated_at TIMESTAMP DEFAULT NOW()
);

-- DOWN
-- automatic rollback generation is not available for semantic schema diffs
