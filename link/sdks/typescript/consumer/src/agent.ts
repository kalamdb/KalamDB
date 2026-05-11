import type {
  AgentChangeParser,
  AgentConnectionRetryPolicy,
  AgentLLMAdapter,
  AgentLLMContext,
  AgentLLMInput,
  ConsumePayload,
  ConsumerChange,
  ConsumerRunContext,
  ConsumerFailureContext,
  ConsumerRunMessage,
  AgentRetryPolicy,
  AgentRunKeyFactory,
  ConsumeContext,
  ConsumeMessage,
  LangChainChatModelLike,
  RunAgentOptions,
  RunConsumerOptions,
  UserId,
} from './types.js';

const DEFAULT_RETRY: Required<Omit<AgentRetryPolicy, 'shouldRetry'>> & {
  shouldRetry: NonNullable<AgentRetryPolicy['shouldRetry']>;
} = {
  maxAttempts: 3,
  initialBackoffMs: 300,
  maxBackoffMs: 5000,
  multiplier: 2,
  jitterRatio: 0,
  shouldRetry: () => true,
};

type NormalizedConnectionRetryPolicy = Required<Omit<AgentConnectionRetryPolicy, 'maxAttempts' | 'shouldRetry'>> & {
  maxAttempts: number | undefined;
  shouldRetry: NonNullable<AgentConnectionRetryPolicy['shouldRetry']>;
};

type BackoffPolicy = {
  initialBackoffMs: number;
  maxBackoffMs: number;
  multiplier: number;
  jitterRatio: number;
};

const DEFAULT_CONNECTION_RETRY: NormalizedConnectionRetryPolicy = {
  enabled: true,
  maxAttempts: undefined,
  initialBackoffMs: 500,
  maxBackoffMs: 30_000,
  multiplier: 1.8,
  jitterRatio: 0.2,
  shouldRetry: () => true,
};

function normalizeRetryPolicy(retry: AgentRetryPolicy | undefined): Required<AgentRetryPolicy> {
  const maxAttempts = Math.max(1, Math.floor(retry?.maxAttempts ?? DEFAULT_RETRY.maxAttempts));
  const initialBackoffMs = Math.max(0, Math.floor(retry?.initialBackoffMs ?? DEFAULT_RETRY.initialBackoffMs));
  const maxBackoffMs = Math.max(initialBackoffMs, Math.floor(retry?.maxBackoffMs ?? DEFAULT_RETRY.maxBackoffMs));
  const multiplier = Math.max(1, retry?.multiplier ?? DEFAULT_RETRY.multiplier);
  const jitterRatio = Math.min(1, Math.max(0, retry?.jitterRatio ?? DEFAULT_RETRY.jitterRatio));

  return {
    maxAttempts,
    initialBackoffMs,
    maxBackoffMs,
    multiplier,
    jitterRatio,
    shouldRetry: retry?.shouldRetry ?? DEFAULT_RETRY.shouldRetry,
  };
}

function normalizeConnectionRetryPolicy(retry: AgentConnectionRetryPolicy | undefined): NormalizedConnectionRetryPolicy {
  const maxAttempts = retry?.maxAttempts === undefined
    ? DEFAULT_CONNECTION_RETRY.maxAttempts
    : Math.max(1, Math.floor(retry.maxAttempts));
  const initialBackoffMs = Math.max(0, Math.floor(retry?.initialBackoffMs ?? DEFAULT_CONNECTION_RETRY.initialBackoffMs));
  const maxBackoffMs = Math.max(initialBackoffMs, Math.floor(retry?.maxBackoffMs ?? DEFAULT_CONNECTION_RETRY.maxBackoffMs));
  const multiplier = Math.max(1, retry?.multiplier ?? DEFAULT_CONNECTION_RETRY.multiplier);
  const jitterRatio = Math.min(1, Math.max(0, retry?.jitterRatio ?? DEFAULT_CONNECTION_RETRY.jitterRatio));

  return {
    enabled: retry?.enabled ?? DEFAULT_CONNECTION_RETRY.enabled,
    maxAttempts,
    initialBackoffMs,
    maxBackoffMs,
    multiplier,
    jitterRatio,
    shouldRetry: retry?.shouldRetry ?? DEFAULT_CONNECTION_RETRY.shouldRetry,
  };
}

function backoffMsForAttempt(
  attempt: number,
  policy: BackoffPolicy,
): number {
  if (attempt <= 1 || policy.initialBackoffMs <= 0) {
    return 0;
  }

  const exponent = attempt - 2;
  const base = policy.initialBackoffMs * (policy.multiplier ** exponent);
  const clamped = Math.min(policy.maxBackoffMs, Math.floor(base));

  if (policy.jitterRatio <= 0) {
    return clamped;
  }

  const jitterWindow = Math.floor(clamped * policy.jitterRatio);
  const min = Math.max(0, clamped - jitterWindow);
  const max = clamped + jitterWindow;
  return Math.floor(min + Math.random() * (max - min + 1));
}

function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  if (ms <= 0 || signal?.aborted) {
    return Promise.resolve();
  }

  return new Promise((resolve) => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const cleanup = () => {
      if (timer !== undefined) {
        clearTimeout(timer);
      }
      signal?.removeEventListener('abort', cleanup);
      resolve();
    };

    timer = setTimeout(cleanup, ms);
    signal?.addEventListener('abort', cleanup, { once: true });
  });
}

function defaultRunKeyFactory({ name, message }: { name: string; message: ConsumeMessage }): string {
  return `${name}:${message.topic}:${message.partition_id}:${message.offset}`;
}

function defaultChangeParser<TPayload extends ConsumePayload>(
  message: ConsumeMessage<TPayload>,
): Record<string, unknown> | null {
  const payload = message.payload ?? message.value;
  if (!payload || typeof payload !== 'object') {
    return null;
  }

  const envelope = payload as Record<string, unknown>;
  const row = envelope.row;
  if (row && typeof row === 'object' && !Array.isArray(row)) {
    return row as Record<string, unknown>;
  }

  return envelope;
}

function normalizeLLMInput(
  input: string | Omit<AgentLLMInput, 'systemPrompt'>,
  systemPrompt: string | undefined,
  runKey: string,
  change: Record<string, unknown>,
): AgentLLMInput {
  if (typeof input === 'string') {
    return {
      prompt: input,
      systemPrompt,
      runKey,
      change,
      row: change,
    };
  }

  return {
    ...input,
    systemPrompt,
    runKey,
    change: input.change ?? change,
    row: input.row ?? change,
  };
}

function createLLMContext(
  llm: AgentLLMAdapter | undefined,
  systemPrompt: string | undefined,
  runKey: string,
  change: Record<string, unknown>,
): AgentLLMContext | null {
  if (!llm) {
    return null;
  }

  return {
    complete: async (input) => llm.complete(normalizeLLMInput(input, systemPrompt, runKey, change)),
    stream: async function* (input) {
      if (!llm.stream) {
        throw new Error('LLM adapter does not support streaming');
      }

      const stream = llm.stream(normalizeLLMInput(input, systemPrompt, runKey, change));
      for await (const chunk of stream) {
        yield chunk;
      }
    },
  };
}

function buildConsumerChange<TData extends Record<string, unknown>, TPayload extends ConsumePayload>(
  data: TData,
  message: ConsumeMessage<TPayload>,
  user: UserId,
): ConsumerChange<TData, TPayload> {
  const contextMessage = buildConsumerRunMessage(message);
  return {
    data,
    message: contextMessage,
    user,
    key: message.key,
    op: message.op,
    timestampMs: message.timestamp_ms,
    partitionId: message.partition_id,
    offset: message.offset,
    topic: message.topic,
    groupId: message.group_id,
  };
}

function resolveConsumerUser<TPayload extends ConsumePayload>(
  consumeCtx: ConsumeContext<TPayload>,
  message: ConsumeMessage<TPayload>,
): UserId {
  const user = consumeCtx.user ?? message.user;
  if (!user) {
    throw new Error(
      'runConsumer: consumed message is missing required user metadata; upgrade the server or republish the topic event with a user id',
    );
  }
  return user;
}

function buildConsumerRunMessage<TPayload extends ConsumePayload>(
  message: ConsumeMessage<TPayload>,
): ConsumerRunMessage<TPayload> {
  const { payload: _payload, value: _value, change: _change, ...metadata } = message as ConsumeMessage<TPayload> & {
    change?: unknown;
  };
  return metadata;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function extractText(value: unknown): string {
  if (typeof value === 'string') {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map(extractText).join('');
  }
  if (!isRecord(value)) {
    return '';
  }
  if (typeof value.text === 'string') {
    return value.text;
  }
  if (typeof value.content === 'string') {
    return value.content;
  }
  if (Array.isArray(value.content)) {
    return value.content
      .map((item) => {
        if (typeof item === 'string') {
          return item;
        }
        if (isRecord(item) && typeof item.text === 'string') {
          return item.text;
        }
        return extractText(item);
      })
      .join('');
  }
  if (isRecord(value.message)) {
    return extractText(value.message);
  }
  return '';
}

function toLangChainInput(input: AgentLLMInput): Array<{ role: string; content: string }> {
  const messages: Array<{ role: string; content: string }> = [];
  const systemPrompt = input.systemPrompt?.trim();
  if (systemPrompt) {
    messages.push({ role: 'system', content: systemPrompt });
  }

  if (input.messages && input.messages.length > 0) {
    for (const message of input.messages) {
      const content = message.content?.trim();
      if (!content) {
        continue;
      }
      messages.push({ role: message.role, content });
    }
  } else if (typeof input.prompt === 'string') {
    const prompt = input.prompt.trim();
    if (prompt) {
      messages.push({ role: 'user', content: prompt });
    }
  }

  return messages;
}

export function createLangChainAdapter(model: LangChainChatModelLike): AgentLLMAdapter {
  return {
    complete: async (input) => {
      const response = await model.invoke(toLangChainInput(input));
      return extractText(response).trim();
    },
    stream: async function* (input) {
      if (!model.stream) {
        throw new Error('Provided LangChain model does not expose stream()');
      }

      const stream = await model.stream(toLangChainInput(input));
      for await (const chunk of stream) {
        const text = extractText(chunk);
        if (text) {
          yield text;
        }
      }
    },
  };
}

export async function runConsumer<
  TData extends Record<string, unknown> = Record<string, unknown>,
  TPayload extends ConsumePayload = TData,
>(
  options: RunConsumerOptions<TData, TPayload>,
): Promise<void> {
  if (!options.name.trim()) {
    throw new Error('runConsumer: name is required');
  }
  if (!options.topic.trim()) {
    throw new Error('runConsumer: topic is required');
  }
  if (!options.groupId.trim()) {
    throw new Error('runConsumer: groupId is required');
  }
  if (!options.onChange && !options.onMessage && !options.onRow) {
    throw new Error('runConsumer: onChange is required');
  }

  const retryPolicy = normalizeRetryPolicy(options.retry);
  const connectionRetryPolicy = normalizeConnectionRetryPolicy(options.connectionRetry);
  const runKeyFactory = options.runKeyFactory ?? defaultRunKeyFactory;
  const changeParser = (options.changeParser ?? options.rowParser ?? defaultChangeParser) as AgentChangeParser<TData, TPayload>;
  const onChange = options.onChange ?? (async (ctx: ConsumerRunContext<TData, TPayload>, change: ConsumerChange<TData, TPayload>) => {
    if (options.onMessage) {
      await options.onMessage(ctx, change);
      return;
    }

    await options.onRow!(ctx, change.data, change);
  });
  const consumerOptions = {
    topic: options.topic,
    group_id: options.groupId,
    start: options.start ?? 'latest',
    batch_size: options.batchSize ?? 20,
    partition_id: options.partitionId ?? 0,
    timeout_seconds: options.timeoutSeconds ?? 30,
    auto_ack: false,
  };
  let pendingConnectionRestoreAttempt: number | null = null;

  const notifyConnectionRestored = () => {
    if (pendingConnectionRestoreAttempt === null) {
      return;
    }

    const attempt = pendingConnectionRestoreAttempt;
    pendingConnectionRestoreAttempt = null;
    options.onConnectionRestored?.({ attempt });
  };

  const runOnce = async (): Promise<void> => {
    const consumer = options.client.consumer<TPayload>(consumerOptions);
    const abortHandler = () => consumer.stop();
    options.stopSignal?.addEventListener('abort', abortHandler, { once: true });

    try {
      await consumer.run(async (consumeCtx) => {
        notifyConnectionRestored();
        const data = changeParser(consumeCtx.message);
        if (!data) {
          await consumeCtx.ack();
          return;
        }
        const message = consumeCtx.message;
        const user = resolveConsumerUser(consumeCtx, message);
        const change = buildConsumerChange<TData, TPayload>(data, message, user);

        const runKey = runKeyFactory({
          name: options.name,
          message,
        });

        let acked = false;
        const ack = async () => {
          if (acked) {
            return;
          }
          await consumeCtx.ack();
          acked = true;
        };

        let lastError: unknown;
        let lastAttempt = 0;
        for (let attempt = 1; attempt <= retryPolicy.maxAttempts; attempt += 1) {
          lastAttempt = attempt;
          const ctx: ConsumerRunContext<TData, TPayload> = {
            name: options.name,
            runKey,
            attempt,
            maxAttempts: retryPolicy.maxAttempts,
            systemPrompt: options.systemPrompt,
            llm: createLLMContext(options.llm, options.systemPrompt, runKey, change.data),
            sql: async (sql, params) => options.client.query(sql, params),
            queryOne: async (sql, params) => options.client.queryOne(sql, params),
            queryAll: async (sql, params) => options.client.queryAll(sql, params),
            ack,
          };

          try {
            await onChange(ctx, change);
            await ack();
            return;
          } catch (error) {
            lastError = error;
            if (acked) {
              options.onError?.({ error, runKey, message: consumeCtx.message });
              return;
            }

            const shouldRetry = attempt < retryPolicy.maxAttempts && retryPolicy.shouldRetry(error, attempt);
            if (!shouldRetry) {
              break;
            }

            const backoffMs = backoffMsForAttempt(attempt + 1, retryPolicy);
            options.onRetry?.({
              error,
              attempt,
              maxAttempts: retryPolicy.maxAttempts,
              backoffMs,
              runKey,
              message: consumeCtx.message,
            });

            if (backoffMs > 0) {
              await sleep(backoffMs, options.stopSignal);
            }
          }
        }

        if (!options.onFailed) {
          options.onError?.({
            error: lastError ?? new Error('Agent message failed with unknown error'),
            runKey,
            message: consumeCtx.message,
          });
          return;
        }

        const failedCtx: ConsumerFailureContext<TData, TPayload> = {
          name: options.name,
          runKey,
          attempt: lastAttempt,
          maxAttempts: retryPolicy.maxAttempts,
          systemPrompt: options.systemPrompt,
          llm: createLLMContext(options.llm, options.systemPrompt, runKey, change.data),
          sql: async (sql, params) => options.client.query(sql, params),
          queryOne: async (sql, params) => options.client.queryOne(sql, params),
          queryAll: async (sql, params) => options.client.queryAll(sql, params),
          ack,
          error: lastError,
        };

        try {
          await options.onFailed(failedCtx, change);
        } catch (failureHandlerError) {
          options.onError?.({
            error: failureHandlerError,
            runKey,
            message: consumeCtx.message,
          });
          return;
        }

        const shouldAckAfterFailure = options.ackOnFailed ?? true;
        if (shouldAckAfterFailure) {
          try {
            await ack();
          } catch (error) {
            options.onError?.({
              error,
              runKey,
              message: consumeCtx.message,
            });
          }
        }
      }, {
        onBatchSuccess: () => {
          notifyConnectionRestored();
        },
      });
    } finally {
      options.stopSignal?.removeEventListener('abort', abortHandler);
    }
  };

  let connectionAttempt = 0;
  while (!options.stopSignal?.aborted) {
    try {
      await runOnce();
      return;
    } catch (error) {
      if (options.stopSignal?.aborted) {
        return;
      }

      connectionAttempt += 1;
      const retryBudgetExhausted = connectionRetryPolicy.maxAttempts !== undefined
        && connectionAttempt >= connectionRetryPolicy.maxAttempts;
      const shouldRetry = connectionRetryPolicy.enabled
        && !retryBudgetExhausted
        && connectionRetryPolicy.shouldRetry(error, connectionAttempt);

      if (!shouldRetry) {
        pendingConnectionRestoreAttempt = null;
        options.onConnectionError?.({ error, attempt: connectionAttempt });
        throw error;
      }

      pendingConnectionRestoreAttempt = connectionAttempt;
      const backoffMs = backoffMsForAttempt(connectionAttempt + 1, connectionRetryPolicy);
      options.onConnectionRetry?.({
        error,
        attempt: connectionAttempt,
        maxAttempts: connectionRetryPolicy.maxAttempts,
        backoffMs,
      });

      await sleep(backoffMs, options.stopSignal);
    }
  }
}

/**
 * @deprecated Use `runConsumer` instead.
 */
export async function runAgent<
  TData extends Record<string, unknown> = Record<string, unknown>,
  TPayload extends ConsumePayload = TData,
>(
  options: RunAgentOptions<TData, TPayload>,
): Promise<void> {
  await runConsumer(options);
}