import {
  KalamDBClient,
  buildAuthHeader,
  resolveAuthProviderWithRetry,
} from '@kalamdb/client';

import type {
  AuthCredentials,
  ClientOptions,
  LoginResponse,
  OnConnectCallback,
  QueryResponse,
  RowData,
  UserId,
} from '@kalamdb/client';
import type {
  AckResponse,
  ConsumePayload,
  ConsumeMessage,
  ConsumeContext,
  ConsumerClientOptions,
  ConsumerConnectionErrorCallback,
  ConsumerConnectionErrorEvent,
  ConsumerConnectCallback,
  ConsumerHandle,
  ConsumerHandler,
  ConsumerOnErrorCallback,
  ConsumerRunLifecycleHooks,
  ConsumeRequest,
  ConsumeResponse,
} from './types.js';
import {
  ConsumerWasmTransport,
  type AckTransportRequest,
  type ConsumeTransportRequest,
  type ConsumeWireMessage,
  type ConsumeWireResponse,
} from './wasm_transport.js';

type TopicStartPayload = 'Latest' | 'Earliest' | { Offset: number };

type TopicAuthCache = {
  sourceKey: string;
  auth: AuthCredentials;
};

const DEFAULT_BATCH_SIZE = 10;
const DEFAULT_IDLE_DELAY_MS = 1000;

class TopicRequestError extends Error {
  readonly status: number;
  readonly code?: string;

  constructor(message: string, status: number, code?: string) {
    super(message);
    this.name = 'TopicRequestError';
    this.status = status;
    this.code = code;
  }
}

const REPORTED_CONNECTION_ERROR = Symbol.for('kalamdb.connectionErrorReported');

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

type TopicErrorLike = {
  message?: unknown;
  status?: unknown;
  code?: unknown;
};

function isTopicErrorLike(value: unknown): value is TopicErrorLike {
  return Boolean(value) && typeof value === 'object';
}

function formatConsumerError(error: unknown): string {
  if (error instanceof Error) {
    const details: string[] = [];
    const errorWithMeta = error as Error & { code?: unknown; cause?: unknown };
    if (typeof errorWithMeta.code === 'string' || typeof errorWithMeta.code === 'number') {
      details.push(`code=${String(errorWithMeta.code)}`);
    }
    if (errorWithMeta.cause !== undefined) {
      details.push(`cause=${formatConsumerError(errorWithMeta.cause)}`);
    }
    return details.length > 0
      ? `${error.message} (${details.join(', ')})`
      : error.message;
  }
  if (typeof error === 'string') {
    return error;
  }
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}

function normalizeConsumeMessage<TPayload extends ConsumePayload>(
  message: ConsumeWireMessage<TPayload>,
): ConsumeMessage<TPayload> {
  return {
    ...message,
    value: message.payload,
  };
}

function normalizeConsumeResponse<TPayload extends ConsumePayload>(
  response: ConsumeWireResponse<TPayload>,
): ConsumeResponse<TPayload> {
  return {
    ...response,
    messages: response.messages.map(normalizeConsumeMessage),
  };
}

function normalizeStart(start: ConsumeRequest['start']): TopicStartPayload {
  if (typeof start === 'number' && Number.isFinite(start)) {
    return { Offset: Math.max(0, Math.floor(start)) };
  }

  if (typeof start === 'string') {
    const normalized = start.trim().toLowerCase();
    if (!normalized || normalized === 'latest') {
      return 'Latest';
    }
    if (normalized === 'earliest') {
      return 'Earliest';
    }
    if (/^\d+$/.test(normalized)) {
      return { Offset: Number.parseInt(normalized, 10) };
    }
    throw new Error(`Invalid consume start value: ${start}`);
  }

  if (start && typeof start === 'object') {
    const offset = 'offset' in start ? start.offset : start.Offset;
    if (typeof offset === 'number' && Number.isFinite(offset)) {
      return { Offset: Math.max(0, Math.floor(offset)) };
    }
  }

  return 'Latest';
}

function isRecoverableConsumerConnectionError(error: unknown): boolean {
  const message = formatConsumerError(error).toLowerCase();
  return !(
    message.includes('401')
    || message.includes('403')
    || message.includes('unauthorized')
    || message.includes('authentication')
    || message.includes('token')
    || message.includes('login failed')
    || message.includes('invalid credentials')
    || message.includes('invalid url')
    || message.includes('relative url')
    || message.includes('configuration error')
    || message.includes('base url')
  );
}

function consumerConnectionHint(message: string, recoverable: boolean, authUser?: string): string {
  const normalized = message.toLowerCase();
  if (
    normalized.includes('invalid url')
    || normalized.includes('relative url')
    || normalized.includes('base url')
    || normalized.includes('url parse')
  ) {
    return 'Check the configured KalamDB URL. Use an absolute http:// or https:// base URL that the worker can reach.';
  }
  if (
    normalized.includes('401')
    || normalized.includes('403')
    || normalized.includes('unauthorized')
    || normalized.includes('authentication')
    || normalized.includes('token')
    || normalized.includes('login failed')
    || normalized.includes('invalid credentials')
  ) {
    return authUser
      ? 'Verify the configured auth user and password or JWT token. Topic consumers using Basic auth must login successfully before polling.'
      : 'Verify the configured JWT token or auth provider before retrying the worker connection.';
  }
  return recoverable
    ? 'Verify KalamDB is running and reachable at the configured URL from this worker, then retry.'
    : 'Review the worker connection configuration and authentication settings.';
}

export class KalamConsumerClient {
  private readonly url: string;
  private readonly sqlClient: KalamDBClient;
  private readonly authProvider: ClientOptions['authProvider'];
  private readonly authProviderMaxAttempts: number;
  private readonly authProviderInitialBackoffMs: number;
  private readonly authProviderMaxBackoffMs: number;
  private readonly topicTransport: ConsumerWasmTransport;
  private cachedTopicAuth: TopicAuthCache | null = null;
  private lastResolvedTopicAuthUser?: string;
  private connectionEstablished = false;
  private connectionAttempt = 0;
  private consumerConnectHandler?: ConsumerConnectCallback;
  private consumerConnectionErrorHandler?: ConsumerConnectionErrorCallback;
  private consumerErrorHandler?: ConsumerOnErrorCallback;
  private readonly reportedErrors = new WeakSet<object>();

  constructor(options: ConsumerClientOptions) {
    if (!options.url) {
      throw new Error('KalamConsumerClient: url is required');
    }
    if (!options.authProvider) {
      throw new Error('KalamConsumerClient: authProvider is required');
    }

    this.url = options.url;

    const {
      consumerWasmUrl: _consumerWasmUrl,
      onConnectionError: consumerOnConnectionError,
      ...baseOptions
    } = options;

    const sqlClientOptions: ClientOptions = {
      ...baseOptions,
      ...(options.onConnectionError
        ? {
            onConnectionError: (error) => {
              consumerOnConnectionError?.({
                ...error,
                error,
                context: 'Base client connection error',
              });
            },
          }
        : {}),
    };

    this.sqlClient = new KalamDBClient(sqlClientOptions);
    this.authProvider = options.authProvider;
    this.authProviderMaxAttempts = options.authProviderMaxAttempts ?? 3;
    this.authProviderInitialBackoffMs = options.authProviderInitialBackoffMs ?? 250;
    this.authProviderMaxBackoffMs = options.authProviderMaxBackoffMs ?? 2000;
    this.consumerConnectHandler = options.onConnect;
    this.consumerConnectionErrorHandler = consumerOnConnectionError;
    this.topicTransport = new ConsumerWasmTransport(
      options.url,
      options.consumerWasmUrl,
    );
  }

  get baseClient(): KalamDBClient {
    return this.sqlClient;
  }

  getAuthType(): 'basic' | 'jwt' | null {
    return this.sqlClient.getAuthType();
  }

  onError(callback: ConsumerOnErrorCallback): void {
    this.consumerErrorHandler = callback;
  }

  onConnect(callback: OnConnectCallback): void {
    this.consumerConnectHandler = callback;
  }

  onConnectionError(callback: ConsumerConnectionErrorCallback): void {
    this.consumerConnectionErrorHandler = callback;
  }

  async query(sql: string, params?: unknown[]): Promise<QueryResponse> {
    return this.sqlClient.query(sql, params);
  }

  async queryOne(sql: string, params?: unknown[]): Promise<RowData | null> {
    return this.sqlClient.queryOne(sql, params);
  }

  async queryAll(sql: string, params?: unknown[]): Promise<RowData[]> {
    return this.sqlClient.queryAll(sql, params);
  }

  async executeAsUser(
    sql: string,
    user: UserId | string,
    params?: unknown[],
  ): Promise<QueryResponse> {
    return this.sqlClient.executeAsUser(sql, user, params);
  }

  async login(): Promise<LoginResponse> {
    const response = await this.sqlClient.login();
    this.cachedTopicAuth = {
      sourceKey: `jwt:${response.access_token}`,
      auth: { type: 'jwt', token: response.access_token },
    };
    return response;
  }

  async refreshToken(refreshToken: string): Promise<LoginResponse> {
    const response = await this.sqlClient.refreshToken(refreshToken);
    this.cachedTopicAuth = {
      sourceKey: `jwt:${response.access_token}`,
      auth: { type: 'jwt', token: response.access_token },
    };
    return response;
  }

  async disconnect(): Promise<void> {
    this.cachedTopicAuth = null;
    await this.sqlClient.disconnect();
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.disconnect();
  }

  [Symbol.dispose](): void {
    void this.disconnect();
  }

  consumer<TPayload extends ConsumePayload = ConsumePayload>(
    options: ConsumeRequest,
  ): ConsumerHandle<TPayload> {
    let stopRequested = false;
    let nextStart = options.start;

    return {
      run: async (
        handler: ConsumerHandler<TPayload>,
        hooks?: ConsumerRunLifecycleHooks,
      ): Promise<void> => {
        stopRequested = false;
        nextStart = options.start;

        try {
          while (!stopRequested) {
            const response = await this.consumeBatch<TPayload>({
              ...options,
              ...(nextStart === undefined ? {} : { start: nextStart }),
            });

            hooks?.onBatchSuccess?.({
              nextOffset: response.next_offset,
              hasMore: response.has_more,
              messageCount: response.messages.length,
            });

            // Keep the server cursor after empty polls so start='latest'
            // continues from the observed high-water mark instead of skipping
            // later inserts by recalculating "latest" on every loop.
            if (response.messages.length === 0) {
              nextStart = { offset: response.next_offset };
            }

            for (const message of response.messages) {
              if (stopRequested) {
                break;
              }

              let acked = false;
              const ctx: ConsumeContext<TPayload> = {
                user: message.user,
                message,
                ack: async () => {
                  if (acked) {
                    return;
                  }
                  acked = true;
                  await this.ack(
                    message.topic,
                    message.group_id,
                    message.partition_id,
                    message.offset,
                  );
                },
              };

              await handler(ctx);

              if (options.auto_ack && !acked) {
                await ctx.ack();
              }
            }

            if (!stopRequested && !response.has_more) {
              await sleep(DEFAULT_IDLE_DELAY_MS);
            }
          }
        } catch (error) {
          hooks?.onError?.(error);
          this.reportConsumerError(
            error,
            `Consumer run failed for topic ${options.topic} group ${options.group_id}`,
          );
          throw error;
        }
      },
      stop: () => {
        stopRequested = true;
      },
    };
  }

  async consumeBatch<TPayload extends ConsumePayload = ConsumePayload>(
    options: ConsumeRequest,
  ): Promise<ConsumeResponse<TPayload>> {
    const request: ConsumeTransportRequest = {
        topic_id: options.topic,
        group_id: options.group_id,
        start: normalizeStart(options.start),
        limit: options.batch_size ?? DEFAULT_BATCH_SIZE,
        partition_id: options.partition_id ?? 0,
        ...(typeof options.timeout_seconds === 'number'
          ? { timeout_seconds: options.timeout_seconds }
          : {}),
      };

    const response = await this.requestTopic<ConsumeWireResponse<TPayload>, ConsumeTransportRequest>(
      request,
      (authHeader, body) => this.topicTransport.consume<TPayload>(authHeader, body),
    );

    return normalizeConsumeResponse(response);
  }

  async ack(
    topic: string,
    groupId: string,
    partitionId: number,
    uptoOffset: number,
  ): Promise<AckResponse> {
    const request: AckTransportRequest = {
        topic_id: topic,
        group_id: groupId,
        partition_id: partitionId,
        upto_offset: uptoOffset,
      };

    return this.requestTopic<AckResponse, AckTransportRequest>(
      request,
      (authHeader, body) => this.topicTransport.ack(authHeader, body),
    );
  }

  private async requestTopic<TResponse, TBody>(
    body: TBody,
    operation: (authHeader: string | undefined, body: TBody) => Promise<TResponse>,
  ): Promise<TResponse> {
    try {
      const response = await this.performTopicRequest(operation, body, false);
      this.notifyConnected();
      return response;
    } catch (error) {
      this.connectionEstablished = false;
      this.connectionAttempt += 1;
      const normalizedError = this.coerceTopicRequestError(error);
      const authUser = this.connectionAuthUser();
      const detail = formatConsumerError(normalizedError);
      const recoverable = isRecoverableConsumerConnectionError(normalizedError);
      const hint = consumerConnectionHint(detail, recoverable, authUser);
      this.reportConnectionError({
        error: normalizedError,
        message: `Topic request failed${authUser ? ` for user "${authUser}"` : ''} at ${this.url}: ${detail}. Hint: ${hint}`,
        recoverable,
        attempt: this.connectionAttempt,
        context: 'Topic request failed',
        url: this.url,
        authUser,
        hint,
      });
      if (!this.isRetryableTopicAuthError(normalizedError)) {
        this.reportConsumerError(normalizedError, 'Topic request failed');
        throw normalizedError;
      }

      try {
        const response = await this.performTopicRequest(operation, body, true);
        this.notifyConnected();
        return response;
      } catch (refreshError) {
        this.connectionEstablished = false;
        this.connectionAttempt += 1;
        const normalizedRefreshError = this.coerceTopicRequestError(refreshError);
        const authUser = this.connectionAuthUser();
        const detail = formatConsumerError(normalizedRefreshError);
        const recoverable = isRecoverableConsumerConnectionError(normalizedRefreshError);
        const hint = consumerConnectionHint(detail, recoverable, authUser);
        this.reportConnectionError({
          error: normalizedRefreshError,
          message: `Topic request failed after auth refresh${authUser ? ` for user "${authUser}"` : ''} at ${this.url}: ${detail}. Hint: ${hint}`,
          recoverable,
          attempt: this.connectionAttempt,
          context: 'Topic request failed after auth refresh',
          url: this.url,
          authUser,
          hint,
        });
        this.reportConsumerError(normalizedRefreshError, 'Topic request failed after auth refresh');
        throw normalizedRefreshError;
      }
    }
  }

  private async performTopicRequest<TResponse, TBody>(
    operation: (authHeader: string | undefined, body: TBody) => Promise<TResponse>,
    body: TBody,
    forceRefresh: boolean,
  ): Promise<TResponse> {
    const auth = await this.resolveTopicAuth(forceRefresh);
    return operation(buildAuthHeader(auth), body);
  }

  private coerceTopicRequestError(error: unknown): unknown {
    if (error instanceof TopicRequestError) {
      return error;
    }

    if (!isTopicErrorLike(error)) {
      return error;
    }

    const status = typeof error.status === 'number'
      ? error.status
      : typeof error.status === 'string' && /^\d+$/.test(error.status)
        ? Number.parseInt(error.status, 10)
        : undefined;
    if (status === undefined) {
      return error;
    }

    return new TopicRequestError(
      typeof error.message === 'string' ? error.message : `Topic request failed: HTTP ${status}`,
      status,
      typeof error.code === 'string' ? error.code : undefined,
    );
  }

  private async resolveTopicAuth(forceRefresh: boolean): Promise<AuthCredentials> {
    const creds = await resolveAuthProviderWithRetry(this.authProvider, {
      maxAttempts: this.authProviderMaxAttempts,
      initialBackoffMs: this.authProviderInitialBackoffMs,
      maxBackoffMs: this.authProviderMaxBackoffMs,
    });
    this.lastResolvedTopicAuthUser = creds.type === 'basic' ? creds.user : undefined;
    const sourceKey = this.authSourceKey(creds);

    if (!forceRefresh && this.cachedTopicAuth?.sourceKey === sourceKey) {
      return this.cachedTopicAuth.auth;
    }

    const effectiveAuth = await this.normalizeTopicAuth(creds);
    this.cachedTopicAuth = {
      sourceKey,
      auth: effectiveAuth,
    };
    return effectiveAuth;
  }

  private connectionAuthUser(): string | undefined {
    if (this.cachedTopicAuth?.auth.type === 'basic') {
      return this.cachedTopicAuth.auth.user;
    }

    return this.lastResolvedTopicAuthUser;
  }

  private async normalizeTopicAuth(auth: AuthCredentials): Promise<AuthCredentials> {
    if (auth.type !== 'basic') {
      return auth;
    }

    const response = await this.sqlClient.login();
    return { type: 'jwt', token: response.access_token };
  }

  private authSourceKey(auth: AuthCredentials): string {
    switch (auth.type) {
      case 'basic':
        return `basic:${auth.user}:${auth.password}`;
      case 'jwt':
        return `jwt:${auth.token}`;
      default: {
        const exhaustive: never = auth;
        return String(exhaustive);
      }
    }
  }

  private isRetryableTopicAuthError(error: unknown): boolean {
    const normalizedError = this.coerceTopicRequestError(error);
    if (!(normalizedError instanceof TopicRequestError)) {
      return false;
    }

    return normalizedError.status === 401
      || normalizedError.code === 'TOKEN_EXPIRED'
      || normalizedError.code === 'UNAUTHENTICATED';
  }

  private reportConsumerError(error: unknown, context: string): void {
    if (this.isErrorAlreadyReported(error)) {
      return;
    }
    this.markErrorReported(error);

    if (this.consumerErrorHandler) {
      try {
        this.consumerErrorHandler(error);
        return;
      } catch (handlerError) {
        console.error('[KalamConsumerClient] onError handler failed:', handlerError);
      }
    }

    console.error(`[KalamConsumerClient] ${context}: ${formatConsumerError(error)}`, error);
  }

  private notifyConnected(): void {
    if (this.connectionEstablished) {
      return;
    }
    this.connectionEstablished = true;
    this.connectionAttempt = 0;
    this.consumerConnectHandler?.();
  }

  private reportConnectionError(event: ConsumerConnectionErrorEvent): void {
    const handler = this.consumerConnectionErrorHandler;
    if (!handler) {
      return;
    }

    if (this.isErrorAlreadyReported(event.error)) {
      return;
    }
    this.markErrorReported(event.error);

    handler(event);
  }

  private markErrorReported(error: unknown): void {
    if (error && typeof error === 'object') {
      Object.defineProperty(error, REPORTED_CONNECTION_ERROR, {
        value: true,
        configurable: true,
      });
      this.reportedErrors.add(error);
    }
  }

  private isErrorAlreadyReported(error: unknown): boolean {
    return Boolean(
      error
      && typeof error === 'object'
      && (
        this.reportedErrors.has(error)
        || Boolean((error as Record<PropertyKey, unknown>)[REPORTED_CONNECTION_ERROR])
      ),
    );
  }
}

export function createConsumerClient(options: ConsumerClientOptions): KalamConsumerClient {
  return new KalamConsumerClient(options);
}