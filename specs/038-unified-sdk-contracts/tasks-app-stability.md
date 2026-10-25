# Application Experience and Database Stability Tasks (Consolidated)

**Revised:** 2026-10-24. **Status:** Consolidated into [tasks.md](tasks.md) under the Owner Decisions and Approvals in [spec.md](spec.md).

All actionable code and verification tasks from `T094`–`T121` (`T098` overlapping same-key mutation state in `@kalamdb/react`, `T110` per-surface atomicity/isolation/durability guarantee matrix, and `T050` final verification) are tracked directly in [tasks.md](tasks.md) Phases 1–3 so `/speckit-implement` executes one unified task plan.

- [X] T094–T097, T099–T109, T111–T121 Scope, approvals, and CI/release schedule boundaries recorded in [spec.md](spec.md) Owner Decisions 1–6.
- [ ] T098 Tracked in [tasks.md](tasks.md) Phase 3 (`link/sdks/typescript/react/src/hooks/useMutationState.ts`).
- [ ] T110 Tracked in [tasks.md](tasks.md) Phase 3 (`docs/architecture/guarantees.md` and `link/tests/sdk-contracts/guarantee-matrix.json`).
