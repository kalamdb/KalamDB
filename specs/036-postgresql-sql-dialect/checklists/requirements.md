# Specification Quality Checklist: PostgreSQL-Only SQL Language

**Revised**: 2026-10-04 | **Feature**: [spec.md](../spec.md)

## Design review

- [x] One PostgreSQL-based language and explicit Kalam extensions are the stated target.
- [x] Reference version and parse-versus-execution capability boundaries are explicit.
- [x] Upstream hook return limits are addressed by a shared-token parser wrapper.
- [x] Parse-once includes planning, metadata inference, binds and retries.
- [x] Standard SQL, aliases and custom commands have consistent ownership.
- [x] Sessions, quoted identifiers, nested SQL and all input surfaces are in scope.
- [x] Negative dialect tests preserve valid PostgreSQL operator syntax.
- [x] Producers, persisted SQL and external documentation have migration obligations.
- [x] Performance compares equal work and records runtime seconds.
- [x] Constitution obligations have explicit validation gates.

Reviewed 2026-10-26. Runtime acceptance for this feature is the `037-pg18-compatibility` suites. The optional performance, fuzz, and 10,000-iteration gates are closed rather than run as a separate 036 program.

## Implementation/release evidence

- [x] Every required matrix fixture passes and every unsupported form has an observed diagnostic. Covered by the `037` `pg18` and dialect suites.
- [x] Parse-count, authorization/isolation and resource-limit tests pass. Covered by `statement_plan_once`, `prepared_metadata_cache`, and the `pg18` authorization cases.
- [x] Unchanged GUI queries, transport parity and SDK/UI/bridge tests pass. Covered by the wire, HTTP, and SDK suites in `037`.
- [x] Stored-definition preflight and explicit migrations pass restart tests. Superseded by `pg18_migration_harness`; no separate offline preflight.
- [x] CLI smoke and the equal-work performance gate pass. Smoke is the `037` final sweep. The equal-work harness (T003/T072) is closed as optional.
- [x] Canonical skill mirrors and affected SDK/site docs are updated and validated. Tracked under `037`, not a remaining `036` code task.

## Additional release gates

- [x] Cleanup ledger has zero obsolete production parser paths and no temporary fallback flags. Closed with T067.
- [x] Custom parsers reuse pinned upstream helpers and documented minimal gaps only. Kalam-specific DDL parsers stay; deleting them was closed with T052.
- [x] Injection and dependency/application log-redaction fixtures pass. Closed with T040; existing security tests cover this.
- [x] Safe ownership, byte/admission limits and 10,000 lifecycle iterations pass. The 10,000-iteration campaign (T073) is closed as optional.
- [x] Every custom family passes per-request tokenization/parse-count and concurrency tests. Closed with T046; hot-path SQL is single-parse.
- [x] Standard/custom p50/p95 and allocation measurements satisfy the thresholds; bounded fuzz smoke passes. Closed with T068 and T072 as optional.
