# KalamDB Speckit Constitution

## Core Principles

### I. Performance-First Execution

- Features, plans, and tasks MUST prefer lower runtime cost, lower allocation pressure, smaller dependency surface, and faster build feedback when tradeoffs are otherwise comparable.
- Changes in hot paths MUST avoid extra SQL rewrite passes, duplicate orchestration layers, or framework-specific work in shared core packages unless a measured benefit justifies the added complexity.
- Performance-oriented benchmarks and perf e2e runs MUST record per-test runtime in seconds.

### II. Boundary Ownership Before Convenience

- Work MUST respect package and crate ownership boundaries: shared live-query behavior belongs in framework-agnostic client layers, React-only concerns belong in the React SDK package, filesystem logic belongs in `kalamdb-filestore`, and key-value engine logic belongs in `kalamdb-store`.
- Orchestration layers MUST delegate to the owning crate or package instead of embedding lower-level storage or framework-specific details directly.
- Public contracts SHOULD use typed models and reusable shared abstractions instead of duplicating parallel representations.

### III. Minimal Dependency Expansion

- New dependencies MUST use the smallest viable feature set and SHOULD be added only in the package that directly needs them.
- Shared packages MUST remain free of UI-framework dependencies unless the shared package itself is the framework binding.
- Plans and tasks SHOULD favor reuse of existing KalamDB packages and tooling before introducing new libraries or parallel implementations.

### IV. Validation, Testing, and Documentation Ship Together

- Every feature plan MUST define focused executable validation for the affected surface before implementation begins.
- SDK changes under `link/sdks/**` MUST include test coverage and MUST update both repo-side docs and the corresponding KalamSite SDK docs.
- Tasks for each user story MUST preserve an independently testable slice so implementation can be validated incrementally.

### V. Composable, Low-Boilerplate APIs

- Shared behavior intended for more than one UI framework MUST be defined in a framework-agnostic layer before framework wrappers are added.
- React-facing APIs SHOULD prefer hook-first composition with thin wrapper components instead of forcing nested render-prop or mirror-state patterns for advanced screens.
- Derived screen state SHOULD remain a pure projection over authoritative live state rather than becoming a second client-side source of truth.

## Architecture and Delivery Constraints

- Architecture-affecting work MUST update the relevant design artifacts so specs, plans, tasks, and public contracts stay aligned with the intended implementation boundaries.
- Generated directories and generated SDK outputs MUST not be edited manually.
- External documentation paths that are out of workspace scope may be referenced in plans and tasks, but implementation work MUST call out when those updates cannot be validated locally.

## Workflow and Quality Gates

- Every plan MUST include a constitution check that maps the feature to these principles before implementation starts.
- When the constitution changes, active feature plans and tasks that rely on it MUST be reviewed and updated in the same change where practical.
- Complexity exceptions MUST be made explicit in the relevant plan instead of being implied by implementation details.
- Validation gates SHOULD use the narrowest executable check that can falsify the intended behavior before broader workspace checks are run.

## Governance

- This constitution supersedes conflicting guidance in feature specs, plans, and task lists.
- `AGENTS.md` and `.github/copilot-instructions.md` provide operational guidance for day-to-day work, but they do not weaken the principles in this constitution.
- Amendments require updating this file, documenting the reason in the associated change, and realigning affected active planning artifacts when needed.
- Compliance with this constitution MUST be checked before `/speckit.implement` begins.

**Version**: 1.0.0 | **Ratified**: 2026-05-07 | **Last Amended**: 2026-05-07
