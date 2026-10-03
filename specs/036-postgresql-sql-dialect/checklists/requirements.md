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

These marks indicate document coverage only. Runtime acceptance remains unchecked until implementation evidence exists.

## Implementation/release evidence

- [ ] Every required matrix fixture passes and every unsupported form has an observed diagnostic.
- [ ] Parse-count, authorization/isolation and resource-limit tests pass.
- [ ] Unchanged GUI queries, transport parity and SDK/UI/bridge tests pass.
- [ ] Stored-definition preflight and explicit migrations pass restart tests.
- [ ] CLI smoke and the equal-work performance gate pass.
- [ ] Canonical skill mirrors and affected SDK/site docs are updated and validated.

## Additional release gates

- [ ] Cleanup ledger has zero obsolete production parser paths and no temporary fallback flags.
- [ ] Custom parsers reuse pinned upstream helpers and documented minimal gaps only.
- [ ] Injection and dependency/application log-redaction fixtures pass.
- [ ] Safe ownership, byte/admission limits and 10,000 lifecycle iterations pass.
- [ ] Every custom family passes per-request tokenization/parse-count and concurrency tests.
- [ ] Standard/custom p50/p95 and allocation measurements satisfy the thresholds; bounded fuzz smoke passes.
