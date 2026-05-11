import type {
  ClientOptions,
  LoginResponse,
  QueryResponse,
  RowData,
  UserId,
} from '@kalamdb/client';

export type {
  AuthCredentials,
  AuthProvider,
  BasicAuthCredentials,
  ClientOptions,
  JwtAuthCredentials,
  LoginResponse,
  LoginUserInfo,
  QueryResponse,
  RowData,
  UserId,
} from '@kalamdb/client';

export interface ConsumerClientOptions extends ClientOptions {
  /**
   * Explicit URL or buffer for the consumer-only WASM file.
   * - Browser: string URL like '/wasm/kalam_consumer_bg.wasm'
   * - Node.js: BufferSource from fs.readFile
   */
  consumerWasmUrl?: string | BufferSource;
}

export type ConsumeStart = 'latest' | 'earliest' | number | { offset: number } | { Offset: number };

export const TopicOp = {
  Insert: 'Insert',
  Update: 'Update',
  Delete: 'Delete',
} as const;

export type TopicOp = (typeof TopicOp)[keyof typeof TopicOp];

export interface ConsumeRequest {
  topic: string;
  group_id: string;
  start?: ConsumeStart;
  batch_size?: number;
  partition_id?: number;
  timeout_seconds?: number;
  auto_ack?: boolean;
  concurrency_per_partition?: number;
}

export type ConsumePayload = Record<string, unknown>;

export interface ConsumeMessage<TPayload extends ConsumePayload = ConsumePayload> {
  key?: string;
  op?: TopicOp;
  timestamp_ms?: number;
  offset: number;
  partition_id: number;
  topic: string;
  group_id: string;
  user: UserId;
  payload: TPayload;
  /**
   * @deprecated Use `payload` instead.
   * Kept as a compatibility alias while callers migrate from the older SDK shape.
   */
  value: TPayload;
}

export type ConsumerRunMessage<TPayload extends ConsumePayload = ConsumePayload> = Omit<
  ConsumeMessage<TPayload>,
  'payload' | 'value'
>;

export interface ConsumeResponse<TPayload extends ConsumePayload = ConsumePayload> {
  messages: ConsumeMessage<TPayload>[];
  next_offset: number;
  has_more: boolean;
}

export interface AckResponse {
  success: boolean;
  acknowledged_offset: number;
}

export interface ConsumeContext<TPayload extends ConsumePayload = ConsumePayload> {
  readonly user: UserId;
  readonly message: ConsumeMessage<TPayload>;
  ack: () => Promise<void>;
}

export type ConsumerHandler<TPayload extends ConsumePayload = ConsumePayload> = (
  ctx: ConsumeContext<TPayload>,
) => Promise<void>;

export interface ConsumerRunLifecycleHooks {
  onBatchSuccess?: (response: {
    nextOffset: number;
    hasMore: boolean;
    messageCount: number;
  }) => void;
}

export interface ConsumerHandle<TPayload extends ConsumePayload = ConsumePayload> {
  run: (handler: ConsumerHandler<TPayload>, hooks?: ConsumerRunLifecycleHooks) => Promise<void>;
  stop: () => void;
}

export interface ConsumerClientLike {
  query: (sql: string, params?: unknown[]) => Promise<QueryResponse>;
  queryOne: (sql: string, params?: unknown[]) => Promise<RowData | null>;
  queryAll: (sql: string, params?: unknown[]) => Promise<RowData[]>;
  consumer: <TPayload extends ConsumePayload = ConsumePayload>(
    options: ConsumeRequest,
  ) => ConsumerHandle<TPayload>;
}

export type AgentLLMRole = 'system' | 'user' | 'assistant';

export interface AgentLLMMessage {
  role: AgentLLMRole;
  content: string;
}

export interface AgentLLMInput {
  prompt?: string;
  messages?: AgentLLMMessage[];
  systemPrompt?: string;
  runKey?: string;
  change?: Record<string, unknown>;
  /**
   * @deprecated Use `change` instead.
   */
  row?: Record<string, unknown>;
}

export interface AgentLLMAdapter {
  complete: (input: AgentLLMInput) => Promise<string>;
  stream?: (input: AgentLLMInput) => AsyncIterable<string>;
}

export interface AgentLLMContext {
  complete: (input: string | Omit<AgentLLMInput, 'systemPrompt'>) => Promise<string>;
  stream: (input: string | Omit<AgentLLMInput, 'systemPrompt'>) => AsyncIterable<string>;
}

export interface LangChainChatModelLike {
  invoke: (...args: any[]) => Promise<any>;
  stream?: (...args: any[]) => AsyncIterable<any> | Promise<AsyncIterable<any>>;
}

export interface AgentRetryPolicy {
  maxAttempts?: number;
  initialBackoffMs?: number;
  maxBackoffMs?: number;
  multiplier?: number;
  jitterRatio?: number;
  shouldRetry?: (error: unknown, attempt: number) => boolean;
}

export interface AgentConnectionRetryPolicy {
  enabled?: boolean;
  maxAttempts?: number;
  initialBackoffMs?: number;
  maxBackoffMs?: number;
  multiplier?: number;
  jitterRatio?: number;
  shouldRetry?: (error: unknown, attempt: number) => boolean;
}

export interface ConsumerChange<TData extends Record<string, unknown>, TPayload extends ConsumePayload = TData> {
  readonly data: TData;
  readonly message: ConsumerRunMessage<TPayload>;
  readonly user: UserId;
  readonly key: string | undefined;
  readonly op: TopicOp | undefined;
  readonly timestampMs: number | undefined;
  readonly partitionId: number;
  readonly offset: number;
  readonly topic: string;
  readonly groupId: string;
}

export interface ConsumerRunContext<TData extends Record<string, unknown>, TPayload extends ConsumePayload = TData> {
  readonly name: string;
  readonly runKey: string;
  readonly attempt: number;
  readonly maxAttempts: number;
  readonly systemPrompt: string | undefined;
  readonly llm: AgentLLMContext | null;
  sql: (sql: string, params?: unknown[]) => Promise<QueryResponse>;
  queryOne: (sql: string, params?: unknown[]) => Promise<RowData | null>;
  queryAll: (sql: string, params?: unknown[]) => Promise<RowData[]>;
  ack: () => Promise<void>;
}

export type ConsumerChangeHandler<
  TData extends Record<string, unknown>,
  TPayload extends ConsumePayload = TData,
> = (ctx: ConsumerRunContext<TData, TPayload>, change: ConsumerChange<TData, TPayload>) => Promise<void>;

export interface ConsumerFailureContext<TData extends Record<string, unknown>, TPayload extends ConsumePayload = TData>
  extends ConsumerRunContext<TData, TPayload> {
  readonly error: unknown;
}

/**
 * @deprecated Use `ConsumerChange` instead.
 */
export type AgentChange<TData extends Record<string, unknown>, TPayload extends ConsumePayload = TData> = ConsumerChange<TData, TPayload>;

/**
 * @deprecated Use `ConsumerRunContext` instead.
 */
export type AgentContext<TData extends Record<string, unknown>, TPayload extends ConsumePayload = TData> = ConsumerRunContext<TData, TPayload>;

/**
 * @deprecated Use `ConsumerFailureContext` instead.
 */
export type AgentFailureContext<TData extends Record<string, unknown>, TPayload extends ConsumePayload = TData> = ConsumerFailureContext<TData, TPayload>;

export type AgentChangeParser<
  TData extends Record<string, unknown>,
  TPayload extends ConsumePayload = TData,
> = (message: ConsumeMessage<TPayload>) => TData | null;

/**
 * @deprecated Use `AgentChangeParser` instead.
 */
export type AgentRowParser<
  TData extends Record<string, unknown>,
  TPayload extends ConsumePayload = TData,
> = AgentChangeParser<TData, TPayload>;

export type ConsumerChangeParser<
  TData extends Record<string, unknown>,
  TPayload extends ConsumePayload = TData,
> = AgentChangeParser<TData, TPayload>;

export type AgentRunKeyFactory = (args: {
  name: string;
  message: ConsumeMessage;
}) => string;

export type AgentFailureHandler<
  TData extends Record<string, unknown>,
  TPayload extends ConsumePayload = TData,
> = (ctx: ConsumerFailureContext<TData, TPayload>, change: ConsumerChange<TData, TPayload>) => Promise<void>;

export interface RunConsumerOptions<
  TData extends Record<string, unknown> = Record<string, unknown>,
  TPayload extends ConsumePayload = TData,
> {
  client: ConsumerClientLike;
  name: string;
  topic: string;
  groupId: string;
  start?: ConsumeRequest['start'];
  batchSize?: number;
  partitionId?: number;
  timeoutSeconds?: number;
  systemPrompt?: string;
  llm?: AgentLLMAdapter;
  retry?: AgentRetryPolicy;
  connectionRetry?: AgentConnectionRetryPolicy;
  runKeyFactory?: AgentRunKeyFactory;
  /**
    * Optional custom change decoder. When omitted, `runConsumer()` uses KalamDB's
   * default topic decoder and unwraps either `{ row: ... }` CDC envelopes or
   * direct row payloads, so generated ORM row types can be used directly.
   */
  changeParser?: AgentChangeParser<TData, TPayload>;
  /**
   * @deprecated Use `changeParser` instead.
   */
  rowParser?: AgentChangeParser<TData, TPayload>;
  onChange?: ConsumerChangeHandler<TData, TPayload>;
  /**
    * @deprecated Use `onChange(ctx, change)` instead.
   */
    onMessage?: (ctx: ConsumerRunContext<TData, TPayload>, change: ConsumerChange<TData, TPayload>) => Promise<void>;
  /**
   * @deprecated Use `onChange(ctx, change)` and read the changed row from `change.data`.
   */
    onRow?: (ctx: ConsumerRunContext<TData, TPayload>, row: TData, change: ConsumerChange<TData, TPayload>) => Promise<void>;
  onFailed?: AgentFailureHandler<TData, TPayload>;
  ackOnFailed?: boolean;
  stopSignal?: AbortSignal;
  onRetry?: (args: {
    error: unknown;
    attempt: number;
    maxAttempts: number;
    backoffMs: number;
    runKey: string;
    message: ConsumeMessage<TPayload>;
  }) => void;
  onConnectionRetry?: (args: {
    error: unknown;
    attempt: number;
    maxAttempts: number | undefined;
    backoffMs: number;
  }) => void;
  onConnectionRestored?: (args: {
    attempt: number;
  }) => void;
  onConnectionError?: (args: {
    error: unknown;
    attempt: number;
  }) => void;
  onError?: (args: {
    error: unknown;
    runKey: string;
    message: ConsumeMessage<TPayload>;
  }) => void;
}

/**
 * @deprecated Use `RunConsumerOptions` instead.
 */
export type RunAgentOptions<
  TData extends Record<string, unknown> = Record<string, unknown>,
  TPayload extends ConsumePayload = TData,
> = RunConsumerOptions<TData, TPayload>;