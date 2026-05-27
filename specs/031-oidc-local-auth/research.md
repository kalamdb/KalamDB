# Research: Unified OIDC and Local Authentication

## Decision: Use One `[auth]` Configuration Surface

**Decision**: Replace the current split `[auth]` plus `[oauth]` provider-family configuration with a single `[auth]` surface. Local username/password policy and OIDC provider settings are both nested under `[auth]`.

**Rationale**: The feature goal is operator clarity and one source of truth. Keeping external provider settings outside `[auth]` makes it harder for Admin UI, CLI, docs, and tests to agree on which login methods are actually available.

**Alternatives considered**: Keeping `[oauth]` for backward compatibility was rejected because it preserves the split configuration model this feature is meant to remove. Keeping provider-specific nested sections was rejected because OIDC already provides a standard provider abstraction.

## Decision: Support Exactly One OIDC Provider

**Decision**: Model external authentication as one optional OIDC provider configured under `[auth.oidc]`, with issuer, client identifier, scopes, redirect expectations, discovery behavior, and optional endpoint overrides.

**Rationale**: One provider keeps login choices deterministic, simplifies audience validation, removes provider-family conditionals, and matches the requested policy that Google, GitHub, Azure, Firebase, and similar providers should be represented through generic OIDC behavior.

**Alternatives considered**: Multiple OIDC providers were rejected because they require provider selection, multiple audiences, cross-provider user collision rules, and more client UX. Provider-family enums were rejected because they reintroduce the old model under new names.

## Decision: Use `ramosbugs/openidconnect-rs` for OIDC Protocol Work

**Decision**: Use `ramosbugs/openidconnect-rs` via the Rust `openidconnect` crate for external OIDC discovery, typed provider metadata, Authorization Code with PKCE, OAuth 2.0 Device Authorization Grant requests, token exchange, ID-token verification, nonce/state handling, standard OIDC claim extraction, and optional UserInfo calls in backend and CLI code that performs protocol work.

**Rationale**: OIDC is a security-sensitive, well-established protocol. The upstream README lists support for OpenID Connect Core, discovery provider metadata, UserInfo, ID-token verification, token introspection, token revocation, RP-initiated logout, and OAuth 2.0 Device Authorization Grant. The docs expose the concrete primitives needed here: `CoreProviderMetadata::discover_async`, `CoreClient::from_provider_metadata`, `CoreClient::authorize_url`, `PkceCodeChallenge::new_random_sha256`, `CoreClient::exchange_code`, `CoreClient::id_token_verifier`, `CoreClient::set_device_authorization_url`, `CoreClient::exchange_device_code`, and `CoreClient::exchange_device_access_token`. Relying on those APIs reduces implementation time and lowers the chance of subtle protocol bugs compared with custom discovery, JWKS, claim validation, PKCE, and device-flow code.

**Implementation guidance**: Add `openidconnect = { version = "4.0.1", default-features = false }` at the workspace dependency level where feasible, then implement a tiny adapter from the crate's `AsyncHttpClient` interface to KalamDB's existing workspace `reqwest` client. The OIDC HTTP client must disable redirect following to avoid SSRF risks, matching the crate documentation's security warning. If the adapter becomes more complex than using the crate-provided HTTP integration, enable only the minimal `reqwest`/`rustls-tls` features in direct OIDC crates and document the dependency-tree impact.

**Alternatives considered**: Continuing hand-written OIDC discovery/JWKS/token-flow code was rejected because it duplicates a mature protocol library and increases security review surface. Adding a broad OAuth framework in addition to `openidconnect` was rejected because `openidconnect` already re-exports and wraps the OAuth2 primitives needed by this feature.

## Decision: Use Authorization Code With PKCE for Browser Login

**Decision**: Admin UI and browser-capable CLI login use OIDC Authorization Code with PKCE. CLI and backend protocol code should use `openidconnect` for PKCE generation, authorization URL construction, token exchange, and ID-token verification. Browser-side Admin UI code should consume the server's public OIDC metadata and use Web Crypto/browser APIs for the public-client PKCE redirect pieces that cannot run in Rust.

**Rationale**: PKCE is the standard public-client flow for browser and CLI apps. It avoids implicit-flow token leakage in URLs while still allowing clients without a client secret to authenticate safely.

**Alternatives considered**: Implicit flow was rejected because modern OIDC guidance favors Authorization Code with PKCE. Resource-owner password grants were rejected because they require provider passwords to pass through KalamDB clients and are not suitable for centralized identity.

## Decision: Use Device Authorization Grant for Headless and No-IdP-Egress CLI Login

**Decision**: Headless CLI login uses the OAuth 2.0 Device Authorization Grant when the configured OIDC provider advertises or is configured with a device authorization endpoint. When the CLI can reach the provider, the CLI uses `openidconnect` directly: build a `CoreClient`, call `set_device_authorization_url`, call `exchange_device_code().request_async(...)`, print `verification_uri_complete` or `verification_uri` plus `user_code`, then call `exchange_device_access_token(&details).request_async(..., sleep_fn, timeout)`. When the CLI cannot reach the IdP but can reach KalamDB, the CLI uses a KalamDB-brokered device flow: KalamDB server uses the same `openidconnect` device APIs, keeps the provider `device_code` server-side, returns only the verification URL/user code/session handle to the CLI, and returns a KalamDB session token after successful provider polling and identity mapping.

**Rationale**: This is the standardized version of the GitHub Copilot-style CLI login experience requested by the user. Direct device flow works when the CLI host cannot open a browser. Brokered device flow covers the stricter no-IdP-egress case: the CLI only needs access to the KalamDB server, while the server performs provider discovery, device-code issuance, and token polling through `openidconnect`. Both modes avoid inventing a KalamDB-specific out-of-band secret exchange.

**Alternatives considered**: Asking users to paste raw ID tokens was rejected because it is error-prone and unsafe. Direct-only device flow was rejected as incomplete for hosts without IdP egress. A fully custom KalamDB one-time-code service was rejected because it duplicates existing OIDC provider behavior; brokered mode is acceptable because KalamDB only proxies the standard provider device grant and stores short-lived broker state.

## Decision: Enforce Local Authentication Policy at Server and Client Surfaces

**Decision**: Local username/password login is controlled by `[auth.local].enabled`. The server is authoritative and rejects local login when disabled. Admin UI and CLI also consume the public login-options contract to hide or explain unavailable manual login.

**Rationale**: Client-side hiding improves UX, but only server-side policy enforcement prevents bypass through direct API calls.

**Alternatives considered**: UI-only gating was rejected as insecure. Removing local users entirely was rejected because deployments may re-enable local authentication or preserve break-glass accounts.

## Decision: Replace Provider Metadata With Login Options

**Decision**: Replace provider-family metadata responses with a public login-options contract that returns whether local login is enabled and, when configured, the single OIDC public client metadata needed by Admin UI and CLI.

**Rationale**: Clients need login capabilities, not a provider list. A single contract reduces duplicated config interpretation and prevents old provider identifiers from leaking back into UI or CLI behavior.

**Alternatives considered**: Keeping `/auth/oauth/providers` was rejected because the plural provider contract reflects the old multi-provider model. Returning full config was rejected because it risks exposing secrets and server-only validation policy.

## Decision: Migrate by Rejecting Legacy Provider-Specific Config With Clear Diagnostics

**Decision**: Old provider-specific keys such as `[oauth.providers.google]`, `[oauth.providers.github]`, `[oauth.providers.azure]`, and `[oauth.providers.firebase]` should fail configuration validation with migration guidance to `[auth.oidc]`.

**Rationale**: Silent compatibility would keep old behavior alive and make docs/tests ambiguous. A clear startup error is safer than accepting mixed auth policy.

**Alternatives considered**: Automatic migration was rejected for the first implementation because it can hide policy changes and may accidentally trust a different issuer/audience than intended.

## Decision: Validate With Dex as the Representative OIDC Provider

**Decision**: Dex remains the test provider for OIDC browser/token validation and should also be used for direct and brokered device-flow coverage when the selected Dex version exposes a device authorization endpoint. If the pinned Dex image does not expose device authorization, keep Dex for normal OIDC acceptance and add a small local standard device-flow fixture only for the RFC 8628 path exercised through `openidconnect`. Tests must cover local-disabled rejection, repeated external login mapping, invalid issuer/audience/token cases, Admin UI login availability, direct CLI device login, and KalamDB-brokered CLI device login.

**Rationale**: Dex is a standards-compliant OIDC provider already used by the auth tests and can run in Docker/testcontainers. Reusing it avoids adding a second identity provider just for tests.

**Alternatives considered**: Fully mocked OIDC providers were rejected for primary acceptance coverage because they miss discovery, JWKS, and provider behavior. A local device-flow fixture is acceptable only if Dex lacks RFC 8628 support in the pinned image. Real cloud providers were rejected because CI would require external credentials and network-dependent setup.

## Decision: Keep Non-OIDC Dependency Growth Minimal

**Decision**: Treat `openidconnect` as the single OIDC protocol dependency and reuse existing HTTP, JWT, serialization, CLI, and UI dependencies for everything else. Any browser-opening or local loopback helper dependency must be added only to the crate/package that directly needs it, with minimal features.

**Rationale**: The constitution and repo guidance prioritize compile speed, small dependency surfaces, and clear ownership boundaries. One focused OIDC dependency is easier to justify and audit than several partial protocol helpers.

**Alternatives considered**: Adding additional OAuth helper crates for PKCE, device flow, or JWT/JWKS validation was rejected unless implementation proves `openidconnect` does not cover a required capability.
