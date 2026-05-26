import type { CurrentUserResponse, LoginResponse, OidcLoginOptions } from "./api";
import { getApiBaseUrl } from "./backend-url";

const OAUTH_STATE_KEY = "kalamdb.admin.oauth.state";
const EXTERNAL_TOKEN_KEY = "kalamdb.admin.oauth.token";
const DEFAULT_EXTERNAL_TOKEN_TTL_MS = 60 * 60 * 1000;
const OAUTH_STATE_TTL_MS = 10 * 60 * 1000;

interface StoredOAuthState {
  state: string;
  nonce: string;
  codeVerifier: string;
  clientId: string;
  tokenEndpoint: string;
  redirectUri: string;
  returnTo: string;
  createdAt: number;
}

export interface OAuthRedirectResult {
  token: string;
  returnTo: string;
}

interface OidcExchangeResponse {
  access_token?: string;
  error?: string;
  error_description?: string;
}

function browserStorage(kind: "sessionStorage" | "localStorage"): Storage | null {
  if (typeof window === "undefined") {
    return null;
  }
  try {
    return window[kind];
  } catch {
    return null;
  }
}

function currentCrypto(): Crypto | null {
  if (typeof globalThis.crypto !== "undefined") {
    return globalThis.crypto;
  }
  return null;
}

function base64UrlEncode(bytes: Uint8Array): string {
  let binary = "";
  bytes.forEach((byte) => {
    binary += String.fromCharCode(byte);
  });
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function randomBytes(length: number): Uint8Array {
  const bytes = new Uint8Array(length);
  const webCrypto = currentCrypto();
  if (webCrypto && typeof webCrypto.getRandomValues === "function") {
    webCrypto.getRandomValues(bytes);
  } else {
    for (let index = 0; index < bytes.length; index += 1) {
      bytes[index] = Math.floor(Math.random() * 256);
    }
  }
  return bytes;
}

function randomToken(byteLength = 32): string {
  return base64UrlEncode(randomBytes(byteLength));
}

async function sha256Base64Url(input: string): Promise<string> {
  const webCrypto = currentCrypto();
  if (!webCrypto?.subtle) {
    throw new Error("Web Crypto is required for OIDC PKCE login");
  }

  const digest = await webCrypto.subtle.digest("SHA-256", new TextEncoder().encode(input));
  return base64UrlEncode(new Uint8Array(digest));
}

export function safeReturnTo(value: string | null | undefined, fallback = "/dashboard"): string {
  const candidate = value?.trim();
  if (!candidate || !candidate.startsWith("/") || candidate.startsWith("//")) {
    return fallback;
  }
  return candidate;
}

function oauthRedirectUri(provider: OidcLoginOptions): string {
  if (provider.admin_redirect_uri) {
    return provider.admin_redirect_uri;
  }
  return new URL("/ui/oauth/callback", window.location.origin).toString();
}

export async function buildOAuthAuthorizationUrl(
  provider: OidcLoginOptions,
  returnTo: string,
): Promise<string> {
  if (!provider.authorization_endpoint) {
    throw new Error("OIDC provider is missing an authorization endpoint");
  }
  if (!provider.token_endpoint) {
    throw new Error("OIDC provider is missing a token endpoint");
  }

  const state = randomToken();
  const nonce = randomToken();
  const codeVerifier = randomToken(64);
  const codeChallenge = await sha256Base64Url(codeVerifier);
  const redirectUri = oauthRedirectUri(provider);
  const storedState: StoredOAuthState = {
    state,
    nonce,
    codeVerifier,
    clientId: provider.client_id,
    tokenEndpoint: provider.token_endpoint,
    redirectUri,
    returnTo: safeReturnTo(returnTo),
    createdAt: Date.now(),
  };
  browserStorage("sessionStorage")?.setItem(OAUTH_STATE_KEY, JSON.stringify(storedState));

  const url = new URL(provider.authorization_endpoint);
  url.searchParams.set("client_id", provider.client_id);
  url.searchParams.set("redirect_uri", redirectUri);
  url.searchParams.set("response_type", "code");
  url.searchParams.set("scope", provider.scopes.join(" ") || "openid email profile");
  url.searchParams.set("state", state);
  url.searchParams.set("nonce", nonce);
  url.searchParams.set("code_challenge", codeChallenge);
  url.searchParams.set("code_challenge_method", "S256");
  url.searchParams.set("prompt", "select_account");
  return url.toString();
}

export async function consumeOAuthRedirect(hash: string, search: string): Promise<OAuthRedirectResult> {
  const hashParams = new URLSearchParams(hash.replace(/^#/, ""));
  const queryParams = new URLSearchParams(search.replace(/^\?/, ""));
  const error = hashParams.get("error") || queryParams.get("error");
  if (error) {
    const description = hashParams.get("error_description") || queryParams.get("error_description");
    throw new Error(description || error);
  }

  const returnedState = hashParams.get("state") || queryParams.get("state");
  const code = queryParams.get("code") || hashParams.get("code");

  const storage = browserStorage("sessionStorage");
  const stored = storage?.getItem(OAUTH_STATE_KEY);
  storage?.removeItem(OAUTH_STATE_KEY);
  if (!stored) {
    throw new Error("OAuth login state was not found");
  }

  const parsed = JSON.parse(stored) as StoredOAuthState;
  if (!returnedState || parsed.state !== returnedState) {
    throw new Error("OAuth login state did not match");
  }
  if (Date.now() - parsed.createdAt > OAUTH_STATE_TTL_MS) {
    throw new Error("OAuth login state expired");
  }
  if (!code) {
    throw new Error("OAuth provider did not return an authorization code");
  }

  const token = await exchangeAuthorizationCode(code, parsed);

  return {
    token,
    returnTo: safeReturnTo(parsed.returnTo),
  };
}

async function exchangeAuthorizationCode(code: string, state: StoredOAuthState): Promise<string> {
  const response = await fetch(`${getApiBaseUrl()}/auth/oidc/exchange-code`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    credentials: "include",
    body: JSON.stringify({
      code,
      redirect_uri: state.redirectUri,
      code_verifier: state.codeVerifier,
    }),
  });
  const payload = (await response.json().catch(() => ({}))) as OidcExchangeResponse;
  if (!response.ok) {
    throw new Error(payload.error_description || payload.error || "OIDC token exchange failed");
  }

  const token = payload.access_token;
  if (!token) {
    throw new Error("KalamDB did not return an access token");
  }
  return token;
}

export function storeExternalAuthToken(token: string): void {
  browserStorage("sessionStorage")?.setItem(EXTERNAL_TOKEN_KEY, token);
}

export function loadExternalAuthToken(): string | null {
  return browserStorage("sessionStorage")?.getItem(EXTERNAL_TOKEN_KEY) ?? null;
}

export function clearExternalAuthToken(): void {
  browserStorage("sessionStorage")?.removeItem(EXTERNAL_TOKEN_KEY);
}

function decodeBase64UrlJson(segment: string): Record<string, unknown> | null {
  try {
    const normalized = segment.replace(/-/g, "+").replace(/_/g, "/");
    const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, "=");
    return JSON.parse(atob(padded)) as Record<string, unknown>;
  } catch {
    return null;
  }
}

export function externalTokenExpiresAt(token: string): string {
  const payload = decodeBase64UrlJson(token.split(".")[1] ?? "");
  const exp = payload?.exp;
  if (typeof exp === "number" && Number.isFinite(exp)) {
    return new Date(exp * 1000).toISOString();
  }
  return new Date(Date.now() + DEFAULT_EXTERNAL_TOKEN_TTL_MS).toISOString();
}

export function externalLoginResponse(
  token: string,
  currentUser: CurrentUserResponse,
): LoginResponse {
  return {
    user: currentUser.user,
    admin_ui_access: currentUser.admin_ui_access,
    expires_at: externalTokenExpiresAt(token),
    access_token: token,
    refresh_token: "",
    refresh_expires_at: externalTokenExpiresAt(token),
  };
}