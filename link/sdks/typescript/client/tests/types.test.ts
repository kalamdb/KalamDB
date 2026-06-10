/**
 * Type safety tests for KalamDB TypeScript SDK
 * Ensures TypeScript definitions are correct
 * Run with: npx tsc --noEmit tests/types.test.ts
 */

import {
  Auth,
  KalamDBClient,
  MessageType,
  ChangeType,
  SeqId,
  createClient,
  type BatchStatus,
  type QueryResponse,
  type ServerMessage,
  type LiveEventsOptions,
  type Unsubscribe,
  type LoginResponse,
  type LiveOptions,
} from '../src/index.js';

// Test: Constructor with Auth.basic
const client1 = new KalamDBClient({
  url: 'http://localhost:2900',
  authProvider: async () => Auth.basic('user', 'pass'),
});

// Test: Constructor with Auth.jwt
const client2 = new KalamDBClient({
  url: 'http://localhost:2900',
  authProvider: async () => Auth.jwt('eyJhbGci...'),
});

// Test: Factory function
const client3 = createClient({
  url: 'http://localhost:2900',
  authProvider: async () => Auth.basic('admin', 'admin'),
});

// Test: Factory function with wsLazyConnect
const client4 = createClient({
  url: 'http://localhost:2900',
  authProvider: async () => Auth.basic('admin', 'admin'),
  wsLazyConnect: true,
  namespace: 'app',
});

// Test: wsLazyConnect defaults to true — connection is managed internally
const client5 = createClient({
  url: 'http://localhost:2900',
  authProvider: async () => Auth.basic('admin', 'admin'),
  wsLazyConnect: false,
});

const client6 = createClient({
  url: 'http://localhost:2900',
  authProvider: async () => Auth.basic('admin', 'admin'),
  onConnectionError: (error) => {
    const message: string = error.message;
    const recoverable: boolean = error.recoverable;
    console.log(message, recoverable);
  },
});

client6.onConnectionError((error) => {
  const message: string = error.message;
  const recoverable: boolean = error.recoverable;
  console.log(message, recoverable);
});

// Test: getAuthType
const authType: 'basic' | 'jwt' | null = client1.getAuthType();

// Test: Async methods return promises
async function testMethods() {
  const client = createClient({
    url: 'http://localhost:2900',
    authProvider: async () => Auth.basic('user', 'pass'),
  });

  // Connection methods
  const disconnectResult: void = await client.disconnect();
  const isConnected: boolean = client.isConnected();

  // Query methods
  const queryResult: QueryResponse = await client.query('SELECT 1');
  const insertResult: QueryResponse = await client.insert('table', { id: 1 });
  const deleteResult: void = await client.delete('table', '123');

  // Live event methods
  const unsub: Unsubscribe = await client.liveEvents('SELECT * FROM table', (event: ServerMessage) => {
    if (event.type === 'change') {
      const ct = event.change_type;
      const rows = event.rows;
    } else if (event.type === 'initial_data_batch') {
      const rows = event.rows;
      const bc = event.batch_control;
    } else if (event.type === 'error') {
      const code: string = event.code;
      const message: string = event.message;
    }
  });

  // Live events with options
  const opts: LiveEventsOptions = {
    batchSize: 50,
    lastRows: 100,
    from: SeqId.from('42'),
  };
  const unsub2 = await client.liveEvents(
    'SELECT * FROM chat.messages',
    (_event: ServerMessage) => {},
    opts,
  );

  const liveOpts: LiveOptions<Record<string, unknown>> = {
    limit: 50,
    getKey: ['id'],
    onCheckpoint: ({ lastSeqId }) => {
      const seq: SeqId = lastSeqId;
      console.log(seq.toString());
    },
  };
  const unsub3 = await client.live(
    'SELECT * FROM chat.messages',
    () => {},
    liveOpts,
  );

  await unsub();
  await unsub2();
  await unsub3();

  // Subscription management
  const count: number = client.getSubscriptionCount();
  const subbed: boolean = client.isSubscribedTo('table');
  await client.unsubscribeAll();

  // Reconnection
  client.setAutoReconnect(true);
  client.setReconnectDelay(1000, 30000);
  client.setMaxReconnectAttempts(5);

  // Login returns LoginResponse
  const loginResponse: LoginResponse = await client.login();
  const accessToken: string = loginResponse.access_token;
}

// Test: QueryResponse structure
const response: QueryResponse = {
  status: 'success',
  results: [
    {
      schema: [
        { name: 'id', data_type: 'BigInt', index: 0 },
        { name: 'name', data_type: 'Text', index: 1 },
      ],
      rows: [[1, 'test']],
      row_count: 1,
    },
  ],
  took: 15.5,
};

// Test: Error response
const errorResponse: QueryResponse = {
  status: 'error',
  results: [],
  error: {
    code: 'ERR_TABLE_NOT_FOUND',
    message: 'Table does not exist',
  },
};

// Test: Enum values
const _mt: MessageType = MessageType.Change;
const _ct: ChangeType = ChangeType.Insert;
// BatchStatus is a type-only export from WASM (not a runtime value)
const _bs: BatchStatus = 'ready';

console.log('✅ Type definitions are valid');
