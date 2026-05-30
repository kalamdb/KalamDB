# Feature Specification: Unified OIDC and Local Authentication

**Feature Branch**: `031-oidc-local-auth`  
**Created**: May 25, 2026  
**Status**: Draft  
**Input**: User description: "Create a new spec for aligning auth with OpenID/OIDC while still supporting local username/password authentication. Server configuration should use only [auth], support only one OIDC/OpenID provider at a time, remove separately configured providers such as Firebase/Google/GitHub/Azure, allow local authentication to be enabled or disabled, support Admin UI external OIDC login, support CLI browser and headless device-style login, use standards-based OAuth 2.0 Device Authorization Grant behavior, support CLI login when the CLI host cannot reach the IdP directly by brokering through KalamDB, test with Dex, and clean old configuration, tests, and code paths."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Configure One Auth Surface (Priority: P1)

As a server operator, I want every authentication setting to live under a single `[auth]` configuration area so that OIDC and local-login behavior are predictable, auditable, and not split across provider-specific sections.

**Why this priority**: A single configuration surface is the foundation for the rest of the feature. Operators must be able to understand which authentication modes are enabled before users rely on Admin UI or CLI login.

**Independent Test**: Start a server from a configuration that contains only `[auth]`, one OIDC provider definition, and a local-auth toggle, then confirm the advertised login methods match that configuration.

**Acceptance Scenarios**:

1. **Given** a server configuration with one enabled OIDC provider and local authentication enabled, **When** the server starts, **Then** OIDC login and username/password login are both available to clients.
2. **Given** a server configuration with one enabled OIDC provider and local authentication disabled, **When** the server starts, **Then** only OIDC-based login is available and manual username/password login is rejected.
3. **Given** a server configuration that uses older provider-specific sections, **When** validation runs, **Then** the configuration is rejected or reported as deprecated according to the migration policy.

---

### User Story 2 - Sign In Through Admin UI OIDC (Priority: P1)

As an Admin UI user, I want to open the configured external OIDC provider, complete authentication there, return to the Admin UI, and enter the app with the correct role and identity.

**Why this priority**: The Admin UI is a primary operator surface. If local authentication is disabled, external OIDC must still provide a complete login path.

**Independent Test**: Configure a standards-compliant OIDC provider, open the Admin UI login screen, complete provider authentication, and verify the user lands in the Admin UI as the expected account.

**Acceptance Scenarios**:

1. **Given** OIDC is configured and local authentication is disabled, **When** an Admin UI user selects external login and completes provider authentication, **Then** the user returns to the Admin UI as an authenticated user.
2. **Given** OIDC is configured and local authentication is enabled, **When** an Admin UI user opens the login page, **Then** both external login and username/password login are available.
3. **Given** the external provider returns an invalid, expired, or untrusted token, **When** the Admin UI attempts to complete login, **Then** the user remains unauthenticated and sees a clear failure state.

---

### User Story 3 - Sign In Through CLI OIDC (Priority: P2)

As a CLI user, I want the CLI to help me authenticate through the configured OIDC provider either by opening a browser or by showing a login URL and one-time code when a browser is unavailable or the CLI host cannot reach the provider directly.

**Why this priority**: Operators often use the CLI from desktops, remote shells, CI hosts, and headless servers. OIDC-only deployments need a usable CLI login path in all of those environments.

**Independent Test**: Run CLI login in browser-capable, direct headless, and KalamDB-brokered headless modes against a configured OIDC provider and confirm each flow produces an authenticated CLI session.

**Acceptance Scenarios**:

1. **Given** OIDC is configured and a browser is available, **When** a CLI user starts external login, **Then** the CLI opens the provider login flow and resumes after successful authentication.
2. **Given** OIDC is configured and no browser is available, **When** a CLI user starts external login from a host that can reach the provider, **Then** the CLI displays a provider URL and one-time code so the user can authenticate from another device.
3. **Given** OIDC is configured and the CLI host can reach KalamDB but not the provider, **When** a CLI user starts headless external login, **Then** KalamDB brokers the standard provider device flow and the CLI authenticates by talking only to KalamDB.
4. **Given** local authentication is disabled, **When** a CLI user attempts username/password login, **Then** the CLI explains that local login is disabled and offers the configured OIDC login path.

---

### User Story 4 - Preserve Local Authentication When Allowed (Priority: P2)

As a deployment owner, I want local username/password authentication to remain available only when explicitly allowed so that development, break-glass, and simple deployments keep working without weakening OIDC-only environments.

**Why this priority**: Some deployments still need local credentials, but others require centralized identity only. The same product must support both policies clearly.

**Independent Test**: Toggle local authentication on and off and verify username/password login, setup behavior, and user management follow the selected policy.

**Acceptance Scenarios**:

1. **Given** local authentication is enabled, **When** a valid local user signs in with username and password, **Then** the user receives a normal authenticated session.
2. **Given** local authentication is disabled, **When** any local username/password credential is submitted, **Then** the attempt is denied without revealing whether the account exists.
3. **Given** local authentication is disabled, **When** an operator reviews available login methods, **Then** manual local login is not presented as an active option.

---

### User Story 5 - Validate With Dex and Remove Legacy Provider Paths (Priority: P3)

As a maintainer, I want tests and documentation to cover the single-OIDC model with Dex and to remove legacy provider-specific authentication paths so that future changes do not reintroduce split auth behavior.

**Why this priority**: The consolidation must be enforceable in tests and docs after the user-facing flows are working.

**Independent Test**: Run the authentication test suite against Dex and verify that old provider-specific configuration and login paths are absent from accepted examples and user-facing behavior.

**Acceptance Scenarios**:

1. **Given** Dex is configured as the OIDC provider, **When** Admin UI and CLI authentication tests run, **Then** OIDC login succeeds for valid identities and fails for invalid tokens.
2. **Given** legacy provider-specific configuration is present, **When** configuration validation runs, **Then** the old configuration is rejected or produces a clear migration error.
3. **Given** docs and examples are reviewed, **When** users search for authentication provider setup, **Then** they find the single OIDC provider model and local-auth toggle, not separate Google, GitHub, Azure, or Firebase setup paths.

### Edge Cases

- OIDC is enabled but issuer, client identifier, callback URL, or provider discovery information is missing or inconsistent.
- More than one OIDC provider is configured at the same time.
- Local authentication is disabled before any OIDC administrator can authenticate.
- A user starts Admin UI or CLI OIDC login but cancels, times out, or returns with an invalid state value.
- A CLI user starts browser login from a non-interactive or remote environment where opening a browser is impossible.
- A CLI user starts headless login from a host that can reach KalamDB but cannot reach the OIDC provider directly.
- A valid OIDC token maps to an existing local username or email but a different external identity.
- A token is valid for a different audience, issuer, tenant, or environment.
- Existing deployments still have old provider-specific settings during upgrade.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST define authentication behavior from a single `[auth]` configuration area for local authentication, OIDC authentication, token validation, and client-facing login availability.
- **FR-002**: The system MUST support at most one configured OIDC/OpenID Connect provider at a time.
- **FR-003**: The system MUST remove user-facing support for separately configured provider families such as Google, GitHub, Azure, and Firebase in favor of the single OIDC provider model.
- **FR-004**: The system MUST expose whether local username/password authentication is allowed or disallowed so Admin UI and CLI clients can present only valid login choices.
- **FR-005**: When local authentication is disabled, the system MUST reject all username/password login attempts for Admin UI, CLI, and direct clients.
- **FR-006**: When local authentication is enabled, the system MUST preserve successful username/password login for authorized local users.
- **FR-007**: The system MUST validate OIDC tokens against the configured issuer, expected audience, signing keys, token lifetime, and identity claims before granting access.
- **FR-008**: The system MUST map a validated OIDC identity to a stable KalamDB user identity and role without creating duplicate users for repeated logins from the same external subject.
- **FR-009**: The Admin UI MUST offer external OIDC login when OIDC is configured and MUST complete login after the provider returns a valid authentication result.
- **FR-010**: The Admin UI MUST hide or disable username/password login when local authentication is disabled.
- **FR-011**: The CLI MUST support an external OIDC login flow that opens a browser when possible and resumes the CLI session after successful authentication.
- **FR-012**: The CLI MUST support a headless external login flow that displays a provider URL and one-time code for users who cannot open a browser on the CLI host, including direct provider device flow and KalamDB-brokered device flow for hosts without direct IdP egress.
- **FR-013**: The CLI MUST clearly report when local username/password login is unavailable because local authentication is disabled.
- **FR-014**: The system MUST provide clear migration feedback for old authentication configuration keys and provider-specific examples.
- **FR-015**: Authentication tests MUST cover the single OIDC provider model with Dex as the current representative external provider.
- **FR-016**: Documentation and examples MUST describe only the unified `[auth]` configuration, the single OIDC provider, the local-auth toggle, Admin UI external login, and CLI browser/headless login flows.
- **FR-017**: Security-sensitive failures MUST use generic user-facing messages while preserving enough operator-facing detail for audit and troubleshooting.
- **FR-018**: The system MUST use standards-compliant OIDC/OAuth2 flows for discovery, PKCE, device authorization, token exchange, and ID-token verification instead of custom out-of-band login semantics.

### Key Entities *(include if feature involves data)*

- **Auth Configuration**: The complete authentication policy for a deployment, including local-login allowance, OIDC provider details, token-validation expectations, and client-visible login capabilities.
- **OIDC Provider**: The single external identity provider trusted by the deployment, including issuer identity, client identity, authorization behavior, and token validation metadata.
- **Local Credential User**: A user account authenticated directly by KalamDB with username/password when local authentication is allowed.
- **External Identity User**: A user account associated with a validated OIDC issuer and subject, with stable mapping to roles and permissions.
- **CLI Login Session**: The state that lets a CLI login attempt start in browser or headless mode and finish as an authenticated local CLI session.
- **OIDC Device Broker Session**: Short-lived server-side state that lets KalamDB broker a standard provider device flow for CLI hosts that cannot reach the IdP directly.
- **Admin UI Login Session**: The browser state that lets an Admin UI user leave for the provider, return safely, and become authenticated.

### Assumptions

- OpenID is treated as OpenID Connect/OIDC for this feature.
- Deployments may enable OIDC only, local authentication only, or both, but only one OIDC provider may be configured at a time.
- Dex is the acceptance-test provider for now because it is standards-compliant and already available in the test environment.
- Direct device flow requires the CLI host to reach provider device and token endpoints; brokered device flow requires the KalamDB server to reach those endpoints while the CLI reaches only KalamDB.
- Existing local users remain valid when local authentication is enabled after migration.
- Role assignment for first-time OIDC users follows the deployment's configured default or pre-provisioned mapping policy.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 100% of accepted authentication examples use only the unified `[auth]` configuration surface.
- **SC-002**: A deployment with local authentication disabled blocks 100% of username/password login attempts across Admin UI, CLI, and direct login clients.
- **SC-003**: A deployment with local authentication enabled allows valid local users to complete username/password login in under 30 seconds.
- **SC-004**: Admin UI users can complete external OIDC login and land in the authenticated app in under 2 minutes during acceptance testing.
- **SC-005**: CLI users can complete browser-based OIDC login in under 2 minutes and direct or brokered headless code-based login in under 5 minutes during acceptance testing.
- **SC-006**: Repeated valid OIDC logins for the same issuer and subject map to the same KalamDB user in 100% of tested attempts.
- **SC-007**: Dex-backed authentication tests cover Admin UI login availability, CLI browser login, CLI direct headless login, CLI brokered headless login, token validation, local-auth-disabled rejection, and duplicate-user prevention.
- **SC-008**: Reviews of public docs and examples find zero active setup paths for separate Google, GitHub, Azure, or Firebase provider configuration.
