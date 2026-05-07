/**
 * @kalamdb/client — Official TypeScript/JavaScript client for KalamDB
 *
 * @example
 * ```typescript
 * import { createClient, Auth } from '@kalamdb/client';
 *
 * const client = createClient({
 *   url: 'http://localhost:8080',
 *   authProvider: async () => Auth.basic('admin', 'admin'),
 * });
 *
 * const unsub = await client.liveEvents('SELECT * FROM messages', (event) => {
 *   console.log('Change:', event);
 * });
 *
 * await unsub();
 * await client.disconnect();
 * ```
 *
 * @packageDocumentation
 */

/* ------------------------------------------------------------------ */
/*  Re-exports                                                        */
/* ------------------------------------------------------------------ */

// Auth
export {
  Auth,
  buildAuthHeader,
  encodeBasicAuth,
  isAuthenticated,
  isBasicAuth,
  isJwtAuth,
} from './auth.js';

export type {
  AuthCredentials,
  BasicAuthCredentials,
  JwtAuthCredentials,
  AuthProvider,
} from './auth.js';

// Types & enums
export {
  ChangeType,
  KalamCellValue,
  LogLevel,
  MessageType,
  SeqId,
  UserId,
  wrapRowMap,
} from './types.js';

export type {
  BatchControl,
  BatchStatus,
  ChangeTypeRaw,
  ClientOptions,
  ConnectionError,
  DisconnectReason,
  ErrorDetail,
  FieldFlag,
  FieldFlags,
  HealthCheckResponse,
  HttpVersion,
  JsonValue,
  KalamDataType,
  LogEntry,
  LogListener,
  LoginResponse,
  LoginUserInfo,
  LiveCallback,
  LiveCheckpoint,
  LiveEventsCallback,
  LiveEventsOptions,
  LiveOptions,
  LiveGetKey,
  LiveStreamOptions,
  OnConnectCallback,
  OnDisconnectCallback,
  OnErrorCallback,
  OnReceiveCallback,
  OnSendCallback,
  QueryResponse,
  QueryResult,
  Role,
  ResponseStatus,
  RowData,
  SchemaField,
  ServerMessage,
  SubscriptionErrorEvent,
  SubscriptionInfo,
  TimestampFormat,
  TypedLiveEventsCallback,
  Unsubscribe,
  UploadProgress,
} from './types.js';

// Client
export { createClient, KalamDBClient } from './client.js';

export {
  createLiveQueryDescriptor,
  createRawSqlLiveDescriptor,
  LiveQueryDescriptorError,
  normalizeLiveSql,
} from './live/descriptor.js';

export {
  parseLiveOrderBy,
  projectLiveRows,
} from './live/projection.js';

export { LiveQueryController } from './live/controller.js';

export type {
  LiveQueryDescriptor,
  LiveQueryDescriptorInput,
  LiveQueryDescriptorMode,
  NormalizedLiveSql,
  RawSqlLiveDescriptorOptions,
} from './live/descriptor.js';

export type {
  LiveProjectionDirection,
  LiveProjectionOrder,
  LiveProjectionPlan,
} from './live/projection.js';

export type {
  LiveQueryControllerListener,
  LiveQueryControllerOptions,
  LiveQueryControllerSnapshot,
  LiveQueryControllerStatus,
} from './live/controller.js';

// Query helpers
export {
  normalizeQueryResponse,
  sortColumns,
  SYSTEM_TABLES_ORDER,
} from './helpers/query_helpers.js';

export {
  isLikelyTransientAuthProviderError,
  resolveAuthProviderWithRetry,
} from './helpers/auth_provider_retry.js';

export type {
  AuthProviderRetryOptions,
} from './helpers/auth_provider_retry.js';

// FileRef helpers
export {
  BoundFileRef,
  FileRef,
  KalamRow,
  parseFileRef,
  parseFileRefs,
  wrapRows,
} from './file_ref.js';

export type {
  FileRefContext,
  FileRefData,
} from './file_ref.js';

// WASM bindings (re-exported so advanced users can access low-level API)
export type { KalamClient as WasmKalamClient } from '../wasm/kalam_client.js';

// Default export
export { KalamDBClient as default } from './client.js';
