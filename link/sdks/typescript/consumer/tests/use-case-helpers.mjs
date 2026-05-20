export function makeConsumerMessage(overrides = {}) {
  const payload = overrides.payload ?? {
    id: `row-${overrides.offset ?? 1}`,
    _table: 'app.events',
  };
  const message = {
    offset: 1,
    partition_id: 0,
    topic: 'app.events_cdc',
    group_id: 'app-worker',
    user: 'user-1',
    key: JSON.stringify({ id: payload.id ?? overrides.offset ?? 1 }),
    op: 'Insert',
    timestamp_ms: 1_779_292_800_000 + Number(overrides.offset ?? 0),
    payload,
    value: payload,
    ...overrides,
  };

  if (!Object.hasOwn(overrides, 'value')) {
    message.value = message.payload;
  }

  return message;
}

export function createConsumerScenarioClient(inputMessages, options = {}) {
  const batches = Array.isArray(inputMessages[0]) ? inputMessages : [inputMessages];
  const state = {
    consumerOptions: [],
    ackedOffsets: [],
    queryCalls: [],
    queryOneCalls: [],
    queryAllCalls: [],
    executeAsUserCalls: [],
  };

  const client = {
    query: async (sql, params) => {
      state.queryCalls.push({ sql, params });
      return options.query?.(sql, params, state) ?? { status: 'success', results: [] };
    },
    queryOne: async (sql, params) => {
      state.queryOneCalls.push({ sql, params });
      return options.queryOne?.(sql, params, state) ?? null;
    },
    queryAll: async (sql, params) => {
      state.queryAllCalls.push({ sql, params });
      return options.queryAll?.(sql, params, state) ?? [];
    },
    executeAsUser: async (sql, user, params) => {
      state.executeAsUserCalls.push({ sql, user, params });
      return { status: 'success', results: [] };
    },
    consumer: (consumeOptions) => {
      state.consumerOptions.push(consumeOptions);
      let stopped = false;

      return {
        run: async (handler, hooks) => {
          for (const [batchIndex, batch] of batches.entries()) {
            if (stopped) break;
            hooks?.onBatchSuccess?.({
              nextOffset: batch.length > 0 ? Math.max(...batch.map((message) => message.offset)) + 1 : 0,
              hasMore: batchIndex < batches.length - 1,
              messageCount: batch.length,
            });

            const dispatch = async (message) => {
              await handler({
                user: Object.hasOwn(message, 'consumeCtxUser') ? message.consumeCtxUser : message.user,
                message,
                ack: async () => {
                  if (options.ackShouldThrow) {
                    throw new Error('ack failed');
                  }
                  state.ackedOffsets.push(message.offset);
                },
              });
            };

            if (options.parallel) {
              await Promise.all(batch.map(dispatch));
            } else {
              for (const message of batch) {
                if (stopped) break;
                await dispatch(message);
              }
            }
          }
        },
        stop: () => {
          stopped = true;
        },
      };
    },
  };

  return { client, state };
}

export function sortNumbers(values) {
  return [...values].sort((left, right) => left - right);
}