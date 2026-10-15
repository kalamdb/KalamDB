# Release notes

Product notes for [GitHub Releases](https://github.com/kalamdb/KalamDB/releases). One file per version. Newest first.

When cutting a release, copy the version file into the GitHub release body (from the first heading through the Full Changelog link). Mark prereleases as prereleases on GitHub.

## Versions

- [0.8.0](0.8.0.md) — PostgreSQL session settings and one SQL path for HTTP and pgwire
- [0.7.1](0.7.1.md) — Raft-derived row versions and one persisted owner per shared table
- [0.7.0-beta.0](0.7.0-beta.0.md) — typed nested data, scalar indexes, server functions, everyday CLI

## Release Rehearsal Checklist (0.8 PostgreSQL 18 Compatibility)

Before cutting or signing off a release candidate, rehearse the following against the actual built artifacts (`kalamdb-server` and `kalam` CLI) and record results in `specs/037-pg18-compatibility/contracts/compatibility-matrix.md`:

- [X] **Clean install & disposable project**: Initialize a fresh project (`kalam init`), start a local target (`kalam up` / `kalam dev`), apply PostgreSQL 18 schemas, composite/enum/domain types, tables, and overloaded routines, generate SDK bindings (`kalam schema gen`), and verify SQL + SDK execution (`cli/kalam-cli-e2e/tests/cli/test_project_workflow_pg18.rs`).
- [X] **Upgrade rehearsal**: Load historical v1 and v2 row-storage segments, historical schema versions (`StorageVersionLayout`), and pre-existing catalog definitions across a server upgrade without data loss or type drift (`backend/crates/kalamdb-core/tests/pg18_schema_evolution.rs`).
- [X] **Backup and restore**: Export/backup a database containing custom schemas, composite/enum/domain columns, sequences, and user/shared tables, restore into a clean data directory, and verify schema, typed row contents, sequence values, and catalog OIDs match.
- [X] **Restart durability**: Restart `kalamdb-server` after DDL (`CREATE SCHEMA`, `CREATE TYPE`, `CREATE DOMAIN`, `ALTER TABLE ... ALTER COLUMN TYPE`), DML, and routine publication; verify catalog providers (`pg_catalog.*`, `information_schema.*`), `TypeRegistry` revisions, and stored rows survive restart (`backend/crates/kalamdb-core/tests/pg_type_slice.rs`, `backend/crates/kalamdb-core/tests/pg18_migration_harness.rs`).
- [X] **Interrupted operation recovery**: Inject failures during explicit transactions (`ROLLBACK`, `ROLLBACK TO SAVEPOINT`, `25P02` aborted transaction state), failed `ALTER COLUMN TYPE ... USING` casts, and interrupted or fenced CLI migrations (`cli/tests/pg18_lifecycle_fault_injection.rs`); verify atomic rollback or durable fenced migration receipts with zero partial corruption.

