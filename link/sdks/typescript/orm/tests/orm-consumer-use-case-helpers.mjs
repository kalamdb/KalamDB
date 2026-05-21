export function column(name, dataType, options = {}) {
  return {
    column_name: name,
    ordinal_position: options.ordinal ?? 1,
    data_type: dataType,
    is_nullable: options.nullable ?? true,
    is_primary_key: options.primary ?? false,
    default_value: options.defaultValue ?? 'None',
    ...(options.comment ? { column_comment: options.comment } : {}),
  };
}

export function tableInfo(namespaceId, tableName, tableType, columns) {
  return {
    table_id: `${namespaceId}:${tableName}`,
    table_name: tableName,
    namespace_id: namespaceId,
    table_type: tableType,
    columns: JSON.stringify(columns),
  };
}

export function createSchemaClient(tables) {
  return {
    query: async (sql) => {
      if (sql === 'SHOW TABLES') {
        return { results: [{ named_rows: tables }] };
      }
      return { results: [{ named_rows: [] }] };
    },
  };
}

export function makeOrmMessage(overrides = {}) {
  const payload = overrides.payload ?? { id: `row-${overrides.offset ?? 1}` };
  const message = {
    offset: 1,
    partition_id: 0,
    topic: 'app.events',
    group_id: 'orm-worker',
    user: 'admin',
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

export function createOrmConsumerClient(messages, options = {}) {
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
      return {
        run: async (handler, hooks) => {
          hooks?.onBatchSuccess?.({
            nextOffset: messages.length > 0 ? Math.max(...messages.map((message) => message.offset)) + 1 : 0,
            hasMore: false,
            messageCount: messages.length,
          });
          for (const message of messages) {
            await handler({
              user: message.user,
              message,
              ack: async () => {
                state.ackedOffsets.push(message.offset);
              },
            });
          }
        },
        stop: () => {},
      };
    },
  };

  return { client, state };
}