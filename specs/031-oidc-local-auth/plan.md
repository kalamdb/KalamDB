# Implementation Plan: Unified OIDC and Local Authentication

**Branch**: `031-oidc-local-auth` | **Date**: May 25, 2026 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/031-oidc-local-auth/spec.md`

## Summary

Consolidate KalamDB authentication into a single `[auth]` configuration surface that supports a local username/password policy and exactly one OpenID Connect (OIDC) provider. Remove provider-family configuration such as Google, GitHub, Azure, and Firebase from the user-facing auth model; rely on `ramosbugs/openidconnect-rs` (`openidconnect`) as the Rust OIDC engine for discovery, typed provider metadata, Authorization Code with PKCE, OAuth 2.0 Device Authorization Grant, token exchange, and ID-token validation. Admin UI remains a public browser client that consumes server-provided OIDC metadata, browser-capable CLI login uses Authorization Code with PKCE, headless CLI login uses device flow, and CLI hosts without direct IdP egress use a KalamDB-brokered device flow where the server performs the `openidconnect` device-code and polling calls. Local username/password login remains available only when explicitly allowed.

## Technical Context

**Language/Version**: Rust 1.92+ edition 2021 for backend and CLI; TypeScript 5.x with React 19 and Vite for Admin UI  
**Primary Dependencies**: Actix-Web, tokio, serde, jsonwebtoken for internal KalamDB JWTs, `openidconnect` 4.0.1 from `ramosbugs/openidconnect-rs` for external OIDC protocol work, existing workspace `reqwest` through a redirect-disabled `openidconnect` HTTP adapter, `kalamdb-configs`, `kalamdb-api`, `kalamdb-auth`, `kalamdb-system`, CLI clap/reqwest stack, Redux Toolkit, testcontainers Dex module  
**Storage**: Existing system users table through `kalamdb-system`/EntityStore, existing CLI credentials file at `~/.kalam/credentials.toml`, TOML server configuration; no new database storage engine  
**Testing**: `cargo nextest run`, focused backend auth integration tests, CLI smoke/e2e tests, Dex-backed tests through Docker/testcontainers, Admin UI `npm exec tsc -- --noEmit` and focused Vitest tests  
**Target Platform**: KalamDB server on macOS/Linux, CLI on local and headless shells, Admin UI in modern browsers  
**Project Type**: Multi-surface feature spanning backend web service, CLI, and single-page Admin UI  
**Performance Goals**: Preserve local-login latency, keep `openidconnect` discovery/JWKS metadata cached and bounded, avoid SQL/DML rewrite work in auth paths, avoid unbounded provider discovery calls from public metadata endpoints, and keep brokered device sessions in short-lived bounded memory  
**Constraints**: Single user-facing `[auth]` surface, at most one OIDC provider, local login must be policy-gated everywhere, no generated directory edits, add `openidconnect` with minimal features, generic auth errors for users with operator-visible audit detail, direct CLI device flow requires CLI-to-IdP network while brokered device flow requires only CLI-to-KalamDB plus server-to-IdP network  
**Scale/Scope**: Configuration model, auth initialization, token validation, auth API contracts, Admin UI login, CLI login, Dex tests, docs, examples, and KalamDB skill references

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **Performance-First Execution**: PASS. The plan uses `openidconnect` for protocol correctness, caches provider metadata/JWKS, keeps discovery bounded, uses TTL-bound brokered device sessions, and does not introduce SQL rewrite or query-engine work into auth flows.
- **Boundary Ownership Before Convenience**: PASS. Configuration stays in `kalamdb-configs`; token validation and identity mapping stay in `kalamdb-auth`/`kalamdb-system`; HTTP contracts stay in `kalamdb-api`; Admin UI work stays in `ui`; CLI login work stays in `cli`.
- **Minimal Dependency Expansion**: PASS. Add `openidconnect` only in backend/CLI crates that perform OIDC protocol work, prefer `default-features = false`, and use KalamDB's existing workspace `reqwest` client through the crate's custom/async HTTP client interface so we do not pull a second HTTP stack unless implementation proves it is cheaper.
- **Validation, Testing, and Documentation Ship Together**: PASS. Plan includes Dex-backed backend/CLI validation, Admin UI tests, config migration tests, docs, examples, and KalamDB skill updates.
- **Composable, Low-Boilerplate APIs**: PASS. Shared auth availability is exposed as a small typed contract consumed by Admin UI and CLI rather than duplicating provider-family logic in each client.

No constitution violations are required.

## Project Structure

### Documentation (this feature)

```text
specs/031-oidc-local-auth/
├── plan.md              # This file (/speckit.plan command output)
├── research.md          # Phase 0 output (/speckit.plan command)
├── data-model.md        # Phase 1 output (/speckit.plan command)
├── quickstart.md        # Phase 1 output (/speckit.plan command)
├── contracts/           # Phase 1 output (/speckit.plan command)
└── tasks.md             # Phase 2 output (/speckit.tasks command - NOT created by /speckit.plan)
```

### Source Code (repository root)

```text
backend/crates/kalamdb-configs/src/config/
├── types.rs             # Unified [auth] schema and legacy config validation
├── loader.rs            # Config parsing and migration diagnostics
└── override.rs          # Environment variable overrides for auth settings

backend/crates/kalamdb-auth/src/
├── oidc/client.rs       # openidconnect-backed CoreClient/provider metadata/verifier construction
├── oidc/http.rs         # redirect-disabled openidconnect AsyncHttpClient adapter over workspace reqwest
├── oidc/device.rs       # openidconnect device grant helpers and broker-session state
├── providers/jwt_config.rs # Trusted issuer/audience initialization and openidconnect client cache
└── services/unified/    # Local-vs-OIDC auth policy enforcement

backend/crates/kalamdb-api/src/http/auth/
├── login.rs             # Local login gated by auth policy
├── me.rs                # Current user validation for internal and external tokens
├── login_options.rs     # Public login capability/OIDC metadata endpoint
└── oidc_device.rs       # KalamDB-brokered device-flow start/poll endpoints

backend/tests/misc/auth/
└── test_oidc_auto_provision.rs # Dex-backed OIDC acceptance coverage

cli/src/
├── args/                # Auth command options
├── credentials/         # Token persistence
└── session/             # openidconnect-backed login flow orchestration and auth prompts

cli/tests/
└── smoke/               # CLI local-disabled, OIDC browser, and OIDC device tests

ui/src/
├── components/auth/     # Login form and login availability presentation
├── lib/                 # API and OIDC/PKCE helpers
├── pages/               # Login and OIDC callback pages
└── store/               # Auth state, token storage, refresh/checkAuth behavior

docs/
├── architecture/oidc-authentication.md
├── security/
└── reference/

../kalamdb-skills/
└── skills/kalamdb/references/ # User-facing auth/config workflow updates
```

**Structure Decision**: Use the existing KalamDB multi-crate/backend, CLI, and Admin UI layout. Do not introduce a new auth crate or provider abstraction layer; simplify the existing split by moving provider-family configuration into one OIDC configuration under `[auth]` and updating each consuming surface at its owner boundary.

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

No constitution violations or complexity exceptions are planned.

## Phase 0 Research Summary

See [research.md](./research.md). All planning questions are resolved: unified config shape, `openidconnect` dependency use, single-provider OIDC validation, Admin UI/CLI browser login with PKCE, direct and KalamDB-brokered headless CLI device flow, local-auth policy enforcement, migration cleanup, and Dex validation strategy.

## Phase 1 Design Summary

See [data-model.md](./data-model.md), [contracts/auth-configuration.md](./contracts/auth-configuration.md), [contracts/auth-login-options.md](./contracts/auth-login-options.md), [contracts/cli-auth.md](./contracts/cli-auth.md), [contracts/oidc-device-broker.md](./contracts/oidc-device-broker.md), and [quickstart.md](./quickstart.md).

## Post-Design Constitution Check

- **Performance-First Execution**: PASS. Contracts avoid per-request provider-family scanning; `openidconnect` provider metadata and JWKS remain cached and bounded; brokered device sessions are short-lived and capped.
- **Boundary Ownership Before Convenience**: PASS. Design artifacts assign each change to its owning crate/package and keep generated outputs untouched.
- **Minimal Dependency Expansion**: PASS. Research justifies `openidconnect` as the single OIDC protocol dependency and limits it to direct backend/CLI consumers with minimal features and a small workspace-reqwest adapter.
- **Validation, Testing, and Documentation Ship Together**: PASS. Quickstart defines focused backend, CLI, UI, config, docs, and skills validation.
- **Composable, Low-Boilerplate APIs**: PASS. Login availability is a single typed contract shared by Admin UI and CLI.
