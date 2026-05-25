import type { CurrentUserResponse, LoginResponse, OAuthProviderInfo } from "./api";

const OAUTH_STATE_KEY = "kalamdb.admin.oauth.state";
const EXTERNAL_TOKEN_KEY = "kalamdb.admin.oauth.token";
const DEFAULT_EXTERNAL_TOKEN_TTL_MS = 60 * 60 * 1000;

interface StoredOAuthState {
  state: string;
  nonce: string;
  providerId: string;
  returnTo: string;
  createdAt: number;
}

export interface OAuthRedirectResult {
  token: string;
  returnTo: string;
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

function randomToken(): string {
  const bytes = new Uint8Array(24);
  if (typeof crypto !== "undefined" && typeof crypto.getRandomValues === "function") {
    crypto.getRandomValues(bytes);
  } else {
    for (let index = 0; index < bytes.length; index += 1) {
      bytes[index] = Math.floor(Math.random() * 256);
    }
  }

  return Array.from(bytes)
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

export function buildOAuthAuthorizationUrl(
  provider: OAuthProviderInfo,
  returnTo: string,
): string {
  if (!provider.authorization_endpoint) {
    throw new Error("OAuth provider is missing an authorization endpoint");
  }

  const state = randomToken();
  const nonce = randomToken();
  const redirectUri = new URL("/ui/oauth/callback", window.location.origin).toString();
  const storedState: StoredOAuthState = {
    state,
    nonce,
    providerId: provider.id,
    returnTo,
    createdAt: Date.now(),
  };
  browserStorage("sessionStorage")?.setItem(OAUTH_STATE_KEY, JSON.stringify(storedState));

  const url = new URL(provider.authorization_endpoint);
  url.searchParams.set("client_id", provider.client_id);
  url.searchParams.set("redirect_uri", redirectUri);
  url.searchParams.set("response_type", "id_token");
  url.searchParams.set("scope", provider.scopes.join(" ") || "openid email profile");
  url.searchParams.set("state", state);
  url.searchParams.set("nonce", nonce);
  url.searchParams.set("prompt", "select_account");
  return url.toString();
}

export function consumeOAuthRedirect(hash: string, search: string): OAuthRedirectResult {
  const hashParams = new URLSearchParams(hash.replace(/^#/, ""));
  const queryParams = new URLSearchParams(search.replace(/^\?/, ""));
  const error = hashParams.get("error") || queryParams.get("error");
  if (error) {
    const description = hashParams.get("error_description") || queryParams.get("error_description");
    throw new Error(description || error);
  }

  const returnedState = hashParams.get("state") || queryParams.get("state");
  const token = hashParams.get("id_token") || queryParams.get("id_token") || hashParams.get("access_token") || queryParams.get("access_token");
  if (!token) {
    throw new Error("OAuth provider did not return an ID token");
  }

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

  return {
    token,
    returnTo: parsed.returnTo || "/dashboard",
  };
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