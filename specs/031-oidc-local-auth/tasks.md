# Tasks: Unified OIDC and Local Authentication

**Input**: Design documents from `/specs/031-oidc-local-auth/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Tests**: Tests are included because the specification explicitly requires Dex-backed authentication coverage, CLI OIDC coverage, Admin UI login coverage, migration/config validation, and proof that old provider-specific auth code paths are removed.

**Organization**: Tasks are grouped by user story so each story can be implemented and tested as an independently valuable slice.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel because it touches different files or has no dependency on incomplete tasks
- **[Story]**: Maps to the user story from spec.md; setup, foundational, and polish tasks have no story label
- Every task includes concrete file paths for the implementation agent

## Phase 1: Setup

**Purpose**: Prepare dependency, fixture, module, and cleanup-check scaffolding without changing behavior.

- [X] T001 Add `openidconnect = { version = "4.0.1", default-features = false }` as the single external OIDC protocol dependency with minimal features in Cargo.toml, backend/crates/kalamdb-auth/Cargo.toml, and cli/Cargo.toml
- [X] T002 [P] Create openidconnect-backed auth module scaffolding in backend/crates/kalamdb-auth/src/oidc/client.rs, backend/crates/kalamdb-auth/src/oidc/http.rs, backend/crates/kalamdb-auth/src/oidc/device.rs, and backend/crates/kalamdb-auth/src/oidc/mod.rs
- [X] T003 [P] Create CLI OIDC browser test module placeholders in cli/tests/auth.rs and cli/tests/auth/test_oidc_browser_login.rs
- [X] T004 [P] Create CLI OIDC device-flow test placeholder in cli/tests/auth/test_oidc_device_login.rs
- [X] T005 [P] Create backend local-auth policy test placeholder in backend/tests/misc/auth/test_local_auth_policy.rs
- [X] T006 [P] Create Admin UI OIDC callback test placeholder in ui/src/pages/OAuthCallback.test.tsx
- [X] T007 [P] Create auth cleanup guard script placeholder in scripts/check-auth-oidc-cleanup.sh

---

## Phase 2: Foundational

**Purpose**: Establish shared typed contracts, OIDC client primitives, and fixtures that block all story work.

**Critical**: No user story implementation should begin until these tasks are complete.

- [X] T008 Define public login-options response DTOs in backend/crates/kalamdb-api/src/http/auth/models/login_options.rs
- [X] T009 Define OIDC device-broker response DTOs in backend/crates/kalamdb-api/src/http/auth/models/oidc_device.rs
- [X] T010 Export login-options and device-broker DTOs from backend/crates/kalamdb-api/src/http/auth/models/mod.rs
- [X] T011 Define Admin UI login-options TypeScript types in ui/src/lib/api.ts
- [X] T012 Define CLI login-options model, direct device-flow metadata, broker endpoint model, and fetcher shell in cli/src/session/auth_options.rs
- [X] T013 Implement redirect-disabled `openidconnect` HTTP adapter over workspace `reqwest` in backend/crates/kalamdb-auth/src/oidc/http.rs
- [X] T014 Define openidconnect-backed OIDC client/cache types and custom device endpoint metadata parsing in backend/crates/kalamdb-auth/src/oidc/client.rs
- [X] T015 Define bounded broker-session state types and TTL cleanup hooks in backend/crates/kalamdb-auth/src/oidc/device.rs
- [X] T016 Add reusable Dex OIDC configuration helpers, including device authorization endpoint fixtures modeled on upstream `okta_device_grant`, in backend/tests/misc/auth/test_oidc_auto_provision.rs
- [X] T017 Add reusable CLI auth test helpers for token assertions in cli/tests/common/mod.rs
- [X] T018 Update auth API module declarations for login-options and OIDC device-broker files in backend/crates/kalamdb-api/src/http/auth/mod.rs

**Checkpoint**: Shared contracts, openidconnect primitives, and test fixtures exist; user-story tasks can proceed.

---

## Phase 3: User Story 1 - Configure One Auth Surface (Priority: P1) - MVP

**Goal**: A server configured only with `[auth]` can advertise local/OIDC login availability, validate one OIDC provider, expose device-flow capability, and reject old provider-specific config.

**Independent Test**: Start or parse server configuration containing only `[auth]`, verify login-options output for local-only/OIDC-only/mixed configs, and verify legacy `[oauth.providers.*]` config is rejected with migration guidance.

### Tests for User Story 1

- [X] T019 [P] [US1] Add config parsing tests for `[auth.local]`, `[auth.oidc]`, missing required OIDC fields, and device endpoint overrides in backend/crates/kalamdb-configs/src/config/loader.rs
- [X] T020 [P] [US1] Add environment override tests for unified auth keys in backend/crates/kalamdb-configs/src/config/override.rs
- [X] T021 [P] [US1] Add login-options API contract tests for local-only, OIDC-only, mixed configs, direct device-flow metadata, and brokered device-flow metadata in backend/tests/misc/auth/test_oidc_auto_provision.rs
- [X] T022 [P] [US1] Add legacy `[oauth.providers.*]` rejection tests in backend/tests/misc/auth/test_local_auth_policy.rs
- [X] T023 [P] [US1] Add openidconnect metadata discovery and issuer/audience verifier tests in backend/crates/kalamdb-auth/src/oidc/client.rs

### Implementation for User Story 1

- [X] T024 [US1] Replace split OAuth config structs with unified `[auth.local]` and `[auth.oidc]` settings in backend/crates/kalamdb-configs/src/config/types.rs
- [X] T025 [US1] Implement `[auth]` config loading and legacy `[oauth]` migration diagnostics in backend/crates/kalamdb-configs/src/config/loader.rs
- [X] T026 [US1] Implement unified auth environment overrides in backend/crates/kalamdb-configs/src/config/override.rs
- [X] T027 [US1] Initialize `openidconnect` provider metadata with `CoreProviderMetadata::discover_async`, `CoreClient::from_provider_metadata`, issuer/audience validation, ID-token verifier construction, and custom device endpoint metadata from `[auth.oidc]` in backend/crates/kalamdb-auth/src/oidc/client.rs and backend/crates/kalamdb-auth/src/providers/jwt_config.rs
- [X] T028 [US1] Replace provider-family `init_auth_config` audience registration with single `[auth.oidc]` initialization in backend/crates/kalamdb-auth/src/services/unified/mod.rs
- [X] T029 [US1] Implement `GET /v1/api/auth/login-options` handler in backend/crates/kalamdb-api/src/http/auth/login_options.rs
- [X] T030 [US1] Implement OIDC device-broker start and poll handlers in backend/crates/kalamdb-api/src/http/auth/oidc_device.rs and backend/crates/kalamdb-auth/src/oidc/device.rs
- [X] T031 [US1] Route `GET /v1/api/auth/login-options`, `POST /v1/api/auth/oidc/device/start`, and `POST /v1/api/auth/oidc/device/poll` in backend/crates/kalamdb-api/src/routes.rs
- [X] T032 [US1] Update auth module exports for login-options and OIDC device broker in backend/crates/kalamdb-api/src/http/auth/mod.rs
- [X] T033 [US1] Replace auth examples with unified `[auth]`, `[auth.local]`, and `[auth.oidc]` examples in backend/server.example.toml

**Checkpoint**: User Story 1 is independently testable as the MVP.

---

## Phase 4: User Story 2 - Sign In Through Admin UI OIDC (Priority: P1)

**Goal**: Admin UI shows valid login choices, opens the configured OIDC provider, completes PKCE callback login, and handles invalid callback responses.

**Independent Test**: Mock login-options responses in Admin UI tests, verify username/password controls hide or show correctly, complete a callback with a valid token, and reject invalid state/token responses.

### Tests for User Story 2

- [X] T034 [P] [US2] Add Admin UI login-options tests for OIDC button visibility and local login visibility in ui/src/components/auth/LoginForm.test.tsx
- [X] T035 [P] [US2] Add Web Crypto PKCE/state helper tests for authorization URL and callback parsing in ui/src/lib/oauth.test.ts
- [X] T036 [P] [US2] Add OAuth callback page success and invalid-state tests in ui/src/pages/OAuthCallback.test.tsx
- [X] T037 [P] [US2] Add external-token auth state tests in ui/src/store/authSlice.test.ts

### Implementation for User Story 2

- [X] T038 [US2] Replace Admin UI provider-list API usage with login-options usage in ui/src/lib/api.ts
- [X] T039 [US2] Implement Admin UI Authorization Code with PKCE browser helpers using Web Crypto in ui/src/lib/oauth.ts
- [X] T040 [US2] Update login form to render OIDC external login and local login from login-options in ui/src/components/auth/LoginForm.tsx
- [X] T041 [US2] Update OAuth callback page to exchange and consume PKCE login results in ui/src/pages/OAuthCallback.tsx
- [X] T042 [US2] Wire callback routing and safe return paths in ui/src/App.tsx and ui/src/pages/Login.tsx
- [X] T043 [US2] Update auth state and context for external bearer-token sessions in ui/src/store/authSlice.ts and ui/src/lib/auth.tsx

**Checkpoint**: User Story 2 can be validated with mocked login-options and provider callback inputs.

---

## Phase 5: User Story 3 - Sign In Through CLI OIDC (Priority: P2)

**Goal**: CLI can authenticate through OIDC using browser PKCE login, direct provider device-code login, or KalamDB-brokered device-code login, then persist the accepted bearer token.

**Independent Test**: Run CLI OIDC browser and headless test flows against Dex or the standard device-flow fixture, and verify authenticated CLI commands work after token storage.

### Tests for User Story 3

- [X] T044 [P] [US3] Add CLI browser OIDC smoke test covering `CoreClient::authorize_url` and `exchange_code` behavior in cli/tests/auth/test_oidc_browser_login.rs
- [X] T045 [P] [US3] Add CLI direct device-code OIDC smoke test covering `exchange_device_code` and `exchange_device_access_token` in cli/tests/auth/test_oidc_device_login.rs
- [X] T046 [P] [US3] Add CLI KalamDB-brokered no-IdP-egress device-code smoke test in cli/tests/auth/test_oidc_device_login.rs
- [X] T047 [P] [US3] Add CLI invalid OIDC token storage test in cli/tests/auth/test_oidc_token_validation.rs

### Implementation for User Story 3

- [X] T048 [US3] Add CLI login mode flags for local, OIDC, and no-browser flows in cli/src/args.rs and cli/src/args/parsers.rs
- [X] T049 [US3] Implement CLI login-options fetcher and broker endpoint client in cli/src/session/auth_options.rs
- [X] T050 [US3] Implement CLI browser Authorization Code with PKCE flow using `CoreClient::authorize_url`, `PkceCodeChallenge::new_random_sha256`, `exchange_code`, and `id_token_verifier` in cli/src/session/oidc_browser.rs
- [X] T051 [US3] Implement CLI direct OAuth device-code flow with `set_device_authorization_url`, `exchange_device_code`, and `exchange_device_access_token` in cli/src/session/oidc_device.rs
- [X] T052 [US3] Implement CLI KalamDB-brokered no-IdP-egress device flow against `/v1/api/auth/oidc/device/start` and `/v1/api/auth/oidc/device/poll` in cli/src/session/oidc_device.rs
- [X] T053 [US3] Integrate CLI login method selection into interactive session login in cli/src/session/interactive.rs
- [X] T054 [US3] Persist validated external bearer tokens using existing credential storage in cli/src/credentials.rs and cli/src/session/credentials.rs
- [X] T055 [US3] Register CLI OIDC auth tests in cli/tests/auth.rs

**Checkpoint**: User Story 3 can be tested from the CLI without Admin UI changes.

---

## Phase 6: User Story 4 - Preserve Local Authentication When Allowed (Priority: P2)

**Goal**: Local username/password login continues to work when enabled and is blocked everywhere when disabled.

**Independent Test**: Toggle `[auth.local].enabled`, then verify backend direct login, Admin UI local controls, and CLI local login follow the selected policy.

### Tests for User Story 4

- [X] T056 [P] [US4] Add backend local-enabled and local-disabled login tests in backend/tests/misc/auth/test_local_auth_policy.rs
- [X] T057 [P] [US4] Add CLI local-disabled explanation test in cli/tests/auth/test_oidc_local_policy.rs
- [X] T058 [P] [US4] Add Admin UI local login hidden/visible tests in ui/src/components/auth/LoginForm.test.tsx

### Implementation for User Story 4

- [X] T059 [US4] Enforce local-auth policy in password login handler in backend/crates/kalamdb-api/src/http/auth/login.rs
- [X] T060 [US4] Enforce setup/local bootstrap policy in backend/crates/kalamdb-api/src/http/auth/setup.rs
- [X] T061 [US4] Surface authoritative local-login availability from login-options in backend/crates/kalamdb-api/src/http/auth/login_options.rs
- [X] T062 [US4] Prevent CLI password prompts when local login is disabled in cli/src/session/interactive.rs
- [X] T063 [US4] Hide or disable Admin UI username/password controls from login-options in ui/src/components/auth/LoginForm.tsx

**Checkpoint**: User Story 4 can be validated independently by toggling local authentication.

---

## Phase 7: User Story 5 - Validate With Dex and Remove Legacy Provider Paths (Priority: P3)

**Goal**: Dex acceptance tests cover the single-OIDC model and old provider-specific auth code, custom OIDC/JWKS logic, docs, examples, and skill references are cleaned up.

**Independent Test**: Run the Dex auth suite and cleanup guard scripts to prove valid OIDC works, invalid tokens fail, duplicate users are not created, and old provider-specific or custom OIDC/JWKS paths are absent from active code.

### Tests for User Story 5

- [X] T064 [P] [US5] Add Dex repeated-login and duplicate-user prevention tests in backend/tests/misc/auth/test_oidc_auto_provision.rs
- [X] T065 [P] [US5] Add Dex invalid issuer, invalid audience, and expired token tests in backend/tests/misc/auth/test_oidc_token_validation.rs
- [X] T066 [P] [US5] Add docs/config search check for old auth provider sections in scripts/check-auth-config-docs.sh
- [X] T067 [P] [US5] Add auth-code cleanup guard that fails on `OidcValidator`, `OidcConfig::discover`, `reqwest::get` in auth OIDC code, provider-family branches, or `/auth/oauth/providers` routing in scripts/check-auth-oidc-cleanup.sh

### Implementation for User Story 5

- [X] T068 [US5] Remove provider-specific `OAuthProvidersSettings` and old `[oauth.providers.*]` structs from backend/crates/kalamdb-configs/src/config/types.rs
- [X] T069 [US5] Move shared internal JWT claims out of backend/crates/kalamdb-auth/src/oidc/claims.rs into backend/crates/kalamdb-auth/src/providers/jwt_claims.rs and update backend/crates/kalamdb-auth/src/providers/jwt_auth.rs
- [X] T070 [US5] Replace `JwtConfig` OIDC validator registry with the single openidconnect-backed provider client/verifier cache in backend/crates/kalamdb-auth/src/providers/jwt_config.rs
- [X] T071 [US5] Replace external bearer-token validation with openidconnect ID-token verification and KalamDB identity mapping in backend/crates/kalamdb-auth/src/services/unified/bearer.rs
- [X] T072 [US5] Remove custom OIDC discovery and JWKS modules from backend/crates/kalamdb-auth/src/oidc/config.rs, backend/crates/kalamdb-auth/src/oidc/validator.rs, and backend/crates/kalamdb-auth/src/oidc/utils.rs
- [X] T073 [US5] Narrow OIDC module exports to openidconnect-backed client/http/device/error helpers in backend/crates/kalamdb-auth/src/oidc/mod.rs and backend/crates/kalamdb-auth/src/oidc/error.rs
- [X] T074 [US5] Remove stale `OidcError` conversion paths and update generic auth error mapping in backend/crates/kalamdb-auth/src/errors/error.rs
- [X] T075 [US5] Delete or replace old plural OAuth provider metadata handler in backend/crates/kalamdb-api/src/http/auth/oauth.rs and backend/crates/kalamdb-api/src/http/auth/mod.rs
- [X] T076 [US5] Remove `/v1/api/auth/oauth/providers` routing and replace any route references with `/v1/api/auth/login-options` in backend/crates/kalamdb-api/src/routes.rs
- [X] T077 [US5] Replace provider-family identity classification with generic OIDC issuer/subject storage in backend/crates/kalamdb-commons/src/models/oauth_provider.rs and backend/crates/kalamdb-system/src/providers/users/models/auth_data.rs
- [X] T078 [US5] Remove provider-family auth references from Admin UI API/client code in ui/src/lib/api.ts, ui/src/lib/oauth.ts, and ui/src/components/auth/LoginForm.tsx
- [X] T079 [US5] Update auth docs to the single OIDC model in docs/architecture/oidc-authentication.md, docs/security/README.md, docs/security/backend-hardening.md, docs/security/security-checklist.md, docs/security/firebase-auth.md, and docs/reference/sql.md
- [X] T080 [US5] Update canonical KalamDB skills for auth configuration in ../kalamdb-skills/skills/kalamdb/references/auth.md and ../kalamdb-skills/skills/kalamdb/references/server-configuration.md
- [X] T081 [US5] Audit generated in-repo skill mirrors for old auth config references in .agents/skills/ and update skills-lock.json if skill generation changes it

**Checkpoint**: User Story 5 proves the old provider-family model and duplicated custom OIDC/JWKS implementation are gone from active code, docs, and skills.

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: Final consistency, validation, and cleanup across the completed stories.

- [X] T082 [P] Update implementation command drift in specs/031-oidc-local-auth/quickstart.md
- [X] T083 [P] Update architecture notes for final auth boundary decisions in docs/architecture/oidc-authentication.md and docs/architecture/decisions/adr-015-enum-usage-policy.md
- [X] T084 Run Rust formatting, focused backend checks, and `openidconnect` dependency-tree review documented in specs/031-oidc-local-auth/quickstart.md
- [X] T085 Run Dex-backed backend auth validation documented in specs/031-oidc-local-auth/quickstart.md
- [X] T086 Run CLI auth validation documented in specs/031-oidc-local-auth/quickstart.md
- [X] T087 Run Admin UI typecheck and focused auth tests documented in specs/031-oidc-local-auth/quickstart.md
- [X] T088 Run KalamDB skill build and verify after auth skill edits using ../kalamdb-skills/package.json
- [X] T089 Run final legacy-provider and custom-OIDC cleanup guards from scripts/check-auth-config-docs.sh and scripts/check-auth-oidc-cleanup.sh

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies.
- **Foundational (Phase 2)**: Depends on Setup and blocks all user stories.
- **User Story 1 (Phase 3)**: Depends on Foundational and is the MVP.
- **User Story 2 (Phase 4)**: Depends on Foundational; full end-to-end success depends on User Story 1 login-options API, but UI can be tested independently with mocked options.
- **User Story 3 (Phase 5)**: Depends on Foundational; full end-to-end success depends on User Story 1 login-options API, but CLI flow units can be tested independently with mocked options.
- **User Story 4 (Phase 6)**: Depends on Foundational; integrates with User Story 1 login-options and local policy.
- **User Story 5 (Phase 7)**: Depends on User Stories 1-4 because it removes legacy paths and validates the final system.
- **Polish (Phase 8)**: Depends on all selected user stories.

### User Story Completion Order

1. **US1 Configure One Auth Surface**: MVP and required for real server behavior.
2. **US2 Sign In Through Admin UI OIDC**: P1 UI path; can start after foundational work using mocked login options.
3. **US3 Sign In Through CLI OIDC**: P2 CLI path; can start after foundational work using mocked login options.
4. **US4 Preserve Local Authentication When Allowed**: P2 policy enforcement across backend, CLI, and UI.
5. **US5 Validate With Dex and Remove Legacy Provider Paths**: P3 cleanup and final hardening after active paths are in place.

### Within Each User Story

- Write test tasks first and confirm they fail before implementation.
- Implement typed models and config before handlers or UI/CLI consumers.
- Implement service/helper logic before routing or presentation.
- Remove old custom OIDC/JWKS/provider-family code only after the openidconnect-backed replacement path and tests are in place.
- Complete the checkpoint validation before moving to lower-priority stories.

## Parallel Opportunities

- Setup tasks T003-T007 can run in parallel after T001 dependency review.
- Foundational DTO/model tasks T008-T012 can run in parallel with backend OIDC primitive tasks T013-T015.
- US1 tests T019-T023 can run in parallel before US1 implementation.
- US2 tests T034-T037 can run in parallel because they target separate UI files.
- US3 tests T044-T047 can run in parallel after CLI test scaffolding exists.
- US4 tests T056-T058 can run in parallel because they cover backend, CLI, and UI surfaces.
- US5 tests T064-T067 can run in parallel with docs/skills cleanup after the active model exists.
- Polish tasks T082-T083 can run in parallel before final validation commands T084-T089.

## Parallel Example: User Story 1

```bash
# Configuration, override, API, OIDC client, and legacy rejection tests can be started together:
Task: "T019 Add config parsing tests in backend/crates/kalamdb-configs/src/config/loader.rs"
Task: "T020 Add environment override tests in backend/crates/kalamdb-configs/src/config/override.rs"
Task: "T021 Add login-options API contract tests in backend/tests/misc/auth/test_oidc_auto_provision.rs"
Task: "T022 Add legacy provider rejection tests in backend/tests/misc/auth/test_local_auth_policy.rs"
Task: "T023 Add openidconnect metadata tests in backend/crates/kalamdb-auth/src/oidc/client.rs"
```

## Parallel Example: User Story 3

```bash
# CLI browser, direct device, brokered device, and invalid-token tests can proceed independently:
Task: "T044 Add browser OIDC smoke test in cli/tests/auth/test_oidc_browser_login.rs"
Task: "T045 Add direct device-code OIDC smoke test in cli/tests/auth/test_oidc_device_login.rs"
Task: "T046 Add brokered device-code OIDC smoke test in cli/tests/auth/test_oidc_device_login.rs"
Task: "T047 Add invalid OIDC token storage test in cli/tests/auth/test_oidc_token_validation.rs"
```

## Parallel Example: User Story 5

```bash
# Cleanup validation can be split from docs and model cleanup once openidconnect-backed auth is active:
Task: "T066 Add docs/config search check in scripts/check-auth-config-docs.sh"
Task: "T067 Add auth-code cleanup guard in scripts/check-auth-oidc-cleanup.sh"
Task: "T079 Update auth docs in docs/architecture/oidc-authentication.md"
Task: "T080 Update canonical skill references in ../kalamdb-skills/skills/kalamdb/references/auth.md"
```

## Implementation Strategy

### MVP First

1. Complete Phase 1 setup and Phase 2 foundational contracts/helpers.
2. Complete US1 so the server has one `[auth]` surface, login-options, and openidconnect-backed metadata initialization.
3. Validate US1 independently with config parsing, login-options, and legacy rejection tests.

### Incremental Delivery

1. Add US2 for Admin UI OIDC login using mocked and real login-options.
2. Add US3 for CLI browser, direct device, and brokered device login.
3. Add US4 for local-login policy enforcement across backend, CLI, and UI.
4. Add US5 cleanup only after replacement paths pass, then remove provider-family and custom OIDC/JWKS code.

### Final Validation

1. Run the quickstart backend, CLI, UI, docs, and skills validation tasks.
2. Run cleanup guards to prove old `[oauth.providers.*]`, `/auth/oauth/providers`, `OidcValidator`, custom JWKS fetches, and provider-family branches do not remain in active code.
3. Review dependency tree to confirm `openidconnect` is the only external OIDC protocol dependency and uses the planned feature set.