import { KalamCellValue, type QueryResponse, type QueryResult as SdkQueryResult, type SchemaField } from "@kalamdb/client";
import { executeQuery, getCurrentToken } from "./kalam-client";
import { getApiBaseUrl } from "./backend-url";

// HTTP API client for auth/setup endpoints.
// SQL calls should go through @kalamdb/client in `kalam-client.ts`.

const API_BASE = getApiBaseUrl();
const NO_AUTH_ENDPOINTS = new Set([
  "/auth/login",
  "/auth/refresh",
  "/auth/setup",
  "/auth/status",
  "/auth/login-options",
]);

export interface ApiError {
  error: string;
  message: string;
  details?: Record<string, unknown>;
}

export class ApiClient {
  private baseUrl: string;

  constructor(baseUrl: string = API_BASE) {
    this.baseUrl = baseUrl;
  }

  private async request<T>(
    endpoint: string,
    options: RequestInit = {}
  ): Promise<T> {
    const url = `${this.baseUrl}${endpoint}`;
    const headers = new Headers(options.headers);
    if (!headers.has("Content-Type")) {
      headers.set("Content-Type", "application/json");
    }

    const token = getCurrentToken();
    if (token && !NO_AUTH_ENDPOINTS.has(endpoint) && !headers.has("Authorization")) {
      headers.set("Authorization", `Bearer ${token}`);
    }

    const response = await fetch(url, {
      ...options,
      credentials: "include",
      headers,
    });

    if (!response.ok) {
      const error: ApiError = await response.json().catch(() => ({
        error: "unknown_error",
        message: `Request failed with status ${response.status}`,
      }));
      throw new ApiRequestError(error, response.status);
    }

    // Handle empty responses
    const text = await response.text();
    if (!text) {
      return {} as T;
    }
    return JSON.parse(text);
  }

  async get<T>(endpoint: string): Promise<T> {
    return this.request<T>(endpoint, { method: "GET" });
  }

  async post<T>(endpoint: string, body?: unknown): Promise<T> {
    return this.request<T>(endpoint, {
      method: "POST",
      body: body ? JSON.stringify(body) : undefined,
    });
  }

  async put<T>(endpoint: string, body?: unknown): Promise<T> {
    return this.request<T>(endpoint, {
      method: "PUT",
      body: body ? JSON.stringify(body) : undefined,
    });
  }

  async delete<T>(endpoint: string): Promise<T> {
    return this.request<T>(endpoint, { method: "DELETE" });
  }
}

export class ApiRequestError extends Error {
  constructor(
    public apiError: ApiError,
    public status: number
  ) {
    super(apiError.message);
    this.name = "ApiRequestError";
  }

  get isUnauthorized(): boolean {
    return this.status === 401;
  }

  get isForbidden(): boolean {
    return this.status === 403;
  }
}

// Singleton instance
export const api = new ApiClient();

// SQL execution types
export interface SqlRequest {
  sql: string;
  namespace?: string;
}

export type SqlRow = KalamCellValue[];

// Query result alias for hooks
export interface QueryResult {
  schema: SchemaField[];
  rows: SqlRow[];
  row_count: number;
  truncated: boolean;
  execution_time_ms: number;
  as_user?: string;
}

export interface SqlResponse {
  schema: SchemaField[];
  rows: SqlRow[];
  row_count: number;
  truncated: boolean;
  execution_time_ms: number;
  as_user?: string;
}

function wrapSqlRows(rows: unknown[][] | undefined): SqlRow[] {
  if (!rows || rows.length === 0) {
    return [];
  }

  return rows.map((row) => row.map((value) => KalamCellValue.from(value)));
}

function cellFromNamedRow(row: Record<string, unknown>, field: SchemaField): KalamCellValue {
  return KalamCellValue.from(row[field.name] ?? null);
}

function positionalRowsFromResult(result: SdkQueryResult | undefined, schema: SchemaField[]): SqlRow[] {
  if (!result) {
    return [];
  }

  const namedRows = result.named_rows;
  if (Array.isArray(namedRows) && namedRows.length > 0) {
    return namedRows.map((row) => {
      const record = row as Record<string, unknown>;
      if (schema.length > 0) {
        return schema.map((field) => cellFromNamedRow(record, field));
      }
      return Object.values(record).map((value) => KalamCellValue.from(value));
    });
  }

  const rows = result.rows;
  if (!rows || rows.length === 0) {
    return [];
  }

  const first = rows[0];
  if (first && !Array.isArray(first) && typeof first === "object") {
    return (rows as Record<string, unknown>[]).map((row) =>
      schema.map((field) => cellFromNamedRow(row, field)),
    );
  }

  return wrapSqlRows(rows as unknown[][]);
}

export function sqlResponseFromQuery(response: QueryResponse): SqlResponse {
  if (response.status === "error") {
    throw new Error(response.error?.message ?? "Query failed");
  }

  const result = response.results?.[0];
  const schema = (result?.schema ?? []) as SchemaField[];
  const rows = positionalRowsFromResult(result, schema);
  return {
    schema,
    rows,
    row_count: result?.row_count ?? rows.length,
    truncated: false,
    execution_time_ms: response.took ?? 0,
    as_user: (result as { as_user?: string } | undefined)?.as_user,
  };
}

export async function executeSql(sql: string, namespace?: string): Promise<SqlResponse> {
  if (namespace && namespace.trim().length > 0) {
    console.warn("[api] executeSql namespace parameter is ignored; SQL execution now routes through @kalamdb/client");
  }

  return sqlResponseFromQuery(await executeQuery(sql));
}

// Auth API helpers
export interface LoginRequest {
  user: string;
  password: string;
}

export interface UserInfo {
  id: string;
  username?: string;
  role: string;
  email: string | null;
  created_at: string;
  updated_at: string;
}

export interface LoginResponse {
  user: UserInfo;
  admin_ui_access: boolean;
  expires_at: string;
  access_token: string;
  refresh_token: string;
  refresh_expires_at: string;
}

export interface AuthStatusResponse {
  needs_setup: boolean;
  message?: string;
}

export interface CurrentUserResponse {
  user: UserInfo;
  admin_ui_access: boolean;
}

export interface AuthLoginOptions {
  local: LocalLoginOptions;
  oidc?: OidcLoginOptions | null;
}

export interface LocalLoginOptions {
  enabled: boolean;
}

export interface OidcLoginOptions {
  enabled: boolean;
  display_name: string;
  issuer: string;
  client_id: string;
  authorization_endpoint?: string | null;
  token_endpoint?: string | null;
  device_authorization_endpoint?: string | null;
  scopes: string[];
  admin_redirect_uri?: string | null;
  cli_redirect_uri?: string | null;
  device_flow?: OidcDeviceFlowOptions | null;
}

export interface OidcDeviceFlowOptions {
  direct_supported: boolean;
  broker_supported: boolean;
  device_authorization_endpoint?: string | null;
  broker_start_endpoint?: string | null;
  broker_poll_endpoint?: string | null;
}

export const authApi = {
  status: () => api.get<AuthStatusResponse>("/auth/status"),

  login: (credentials: LoginRequest) =>
    api.post<LoginResponse>("/auth/login", credentials),
  
  logout: () => api.post("/auth/logout"),
  
  refresh: () => api.post<LoginResponse>("/auth/refresh"),
  
  me: () => api.get<CurrentUserResponse>("/auth/me"),

  loginOptions: () => api.get<AuthLoginOptions>("/auth/login-options"),
};

export async function probeBackendReachability(): Promise<AuthStatusResponse> {
  return authApi.status();
}
