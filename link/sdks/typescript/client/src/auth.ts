/**
 * Authentication types and utilities for KalamDB client
 * 
 * Provides a type-safe authentication API with support for:
 * - Basic Auth (user/password)
 * - JWT Token Auth
 * - Dynamic provider (async callback for refresh-token flows)
 */

import { encodeUtf8Base64 } from './helpers/base64.js';

/**
 * Basic authentication credentials (user/password)
 */
export interface BasicAuthCredentials {
  type: 'basic';
  user: string;
  password: string;
}

/**
 * JWT token authentication credentials
 */
export interface JwtAuthCredentials {
  type: 'jwt';
  token: string;
}

/**
 * Union type for all authentication credential types
 */
export type AuthCredentials = BasicAuthCredentials | JwtAuthCredentials;

/**
 * Type guard to check if credentials are Basic Auth
 */
export function isBasicAuth(auth: AuthCredentials): auth is BasicAuthCredentials {
  return auth.type === 'basic';
}

/**
 * Type guard to check if credentials are JWT Auth
 */
export function isJwtAuth(auth: AuthCredentials): auth is JwtAuthCredentials {
  return auth.type === 'jwt';
}

/**
 * Type guard to check if resolved credentials are available
 */
export function isAuthenticated(auth: AuthCredentials | null | undefined): auth is AuthCredentials {
  return auth != null;
}

/**
 * Encode user and password for Basic Auth header
 * 
 * @param user - Canonical user identifier
 * @param password - Password
 * @returns Base64 encoded credentials string
 */
export function encodeBasicAuth(user: string, password: string): string {
  return encodeUtf8Base64(`${user}:${password}`);
}

/**
 * Build the Authorization header value for the given credentials
 * 
 * @param auth - Authentication credentials
 * @returns Authorization header value or undefined when auth is unset
 */
export function buildAuthHeader(auth: AuthCredentials | null | undefined): string | undefined {
  if (!auth) {
    return undefined;
  }

  switch (auth.type) {
    case 'basic':
      throw new Error('User/password credentials are only valid for /v1/api/auth/login. Exchange them for a JWT before sending authenticated requests.');
    case 'jwt':
      return `Bearer ${auth.token}`;
    default:
      // Exhaustiveness check
      const _exhaustive: never = auth;
      throw new Error(`Unknown auth type: ${(_exhaustive as AuthCredentials).type}`);
  }
}

/**
 * Auth factory for creating type-safe authentication credentials
 * 
 * @example
 * ```typescript
 * import { createClient, Auth } from '@kalamdb/client';
 * 
 * // Basic Auth
 * const client = createClient({
 *   url: 'http://localhost:2900',
 *   authProvider: async () => Auth.basic('admin', 'admin')
 * });
 * 
 * // JWT Token
 * const jwtClient = createClient({
 *   url: 'http://localhost:2900',
 *   authProvider: async () => Auth.jwt('eyJhbGciOiJIUzI1NiIs...')
 * });
 * 
 * ```
 */
export const Auth = {
  /**
   * Create Basic Auth credentials
   * 
   * @param user - Canonical user identifier for authentication
   * @param password - Password for authentication
   * @returns BasicAuthCredentials object
   */
  basic(user: string, password: string): BasicAuthCredentials {
    return { type: 'basic', user, password };
  },

  /**
   * Create JWT Auth credentials
   * 
   * @param token - JWT token string
   * @returns JwtAuthCredentials object
   */
  jwt(token: string): JwtAuthCredentials {
    return { type: 'jwt', token };
  }
} as const;

/**
 * Async authentication provider callback.
 *
 * Called before each (re-)connection attempt to obtain fresh credentials.
 * This is the recommended approach for refresh-token flows.
 *
 * Returning `Auth.basic(...)` is supported — the SDK automatically exchanges
 * the credentials through `/v1/api/auth/login` before any authenticated
 * request or WebSocket connection.
 *
 * @example
 * ```typescript
 * const authProvider: AuthProvider = async () => {
 *   const token = await myApp.getOrRefreshJwt();
 *   return Auth.jwt(token);
 * };
 *
 * const client = createClient({
 *   url: 'http://localhost:2900',
 *   authProvider,
 * });
 * ```
 */
export type AuthProvider = () => Promise<AuthCredentials>;

