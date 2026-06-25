-- Migration: draft
-- Updated: 2026-06-25T14:46:20.272222+00:00

-- UP
-- Generated KalamDB schema evolution
-- Review before applying in production.

CREATE NAMESPACE IF NOT EXISTS okf_sync;

CREATE USER TABLE okf_sync.context_files (
  path TEXT PRIMARY KEY,
  file_ref FILE NOT NULL,
  sha256 TEXT NOT NULL,
  base_sha256 TEXT,
  mime_type TEXT NOT NULL,
  size_bytes BIGINT NOT NULL,
  frontmatter JSON,
  is_conflict BOOLEAN DEFAULT false,
  canonical_path TEXT,
  deleted BOOLEAN DEFAULT false,
  created_at TIMESTAMP DEFAULT NOW(),
  updated_at TIMESTAMP DEFAULT NOW()
);

-- DOWN
-- automatic rollback generation is not available for semantic schema diffs
