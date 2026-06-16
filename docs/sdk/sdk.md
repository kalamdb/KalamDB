# KalamDB TypeScript/JavaScript SDK

The official TypeScript/JavaScript SDK for KalamDB, built on top of a Rust -> WASM core.

Worker and topic-consumer APIs now live in the separate `@kalamdb/consumer` package. React live-query UI APIs live in `@kalamdb/react`, which wraps the app-facing `@kalamdb/client` surface.

- **Tiny bundle size** with minimal dependencies
- **Cross-platform**: Works in Node.js and browsers
- **Type-safe**: Full TypeScript support with complete type definitions
- **Real-time**: WebSocket subscriptions with Firebase/Supabase-style API

## Installation

```bash
npm install @kalamdb/client
# or
yarn add @kalamdb/client
# or
pnpm add @kalamdb/client
```

For React live-query components and hooks:

```bash
npm install @kalamdb/client @kalamdb/react react react-dom
npm install @kalamdb/orm drizzle-orm
```

When you use generated Drizzle tables from `@kalamdb/orm`, configure the KalamDB namespace once before importing the generated schema module:

```typescript
// db/kalam-orm.ts
import { configureKalamOrm } from '@kalamdb/orm';

configureKalamOrm({ namespace: 'app' });
```

```typescript
// db/index.ts
import './kalam-orm';
export * from './schema.generated';
```

## Building From Source (This Repo)

This repo contains two related pieces:

- The Rust client crate: `link/sdks/rust/` (package name: `kalam-client`, native only)
- The browser WASM entry crate: `link/kalam-link-wasm/` (built by `@kalamdb/client`)
- The npm-publishable TypeScript SDK package: `link/sdks/typescript/client/`

### Prerequisites

- Rust toolchain (workspace uses Rust stable)
- Node.js `>=18` (see `link/sdks/typescript/client/package.json` engines)
- `wasm-pack` (used to compile Rust → WASM)

Install wasm-pack:

```bash
cargo install wasm-pack
```

### Compile the browser WASM module (`kalam-link-wasm`)

From the repo root:

```bash
wasm-pack build kalam-link-wasm --target web --out-dir link/sdks/typescript/client/wasm --out-name kalam_client --profile release-dist
```

### Compile the native Rust client (`kalam-client`)

From the repo root:

```bash
cargo build -p kalam-client
cargo test -p kalam-client
```

### Build the TypeScript SDK

The SDK build compiles the Rust WASM module with `wasm-pack` and then runs `tsc`.

```bash
cd link/sdks/typescript/client
npm install
npm run build
```

Outputs land in `link/sdks/typescript/client/dist/`.

### Using the SDK locally (monorepo)

In another Node project inside this repo, depend on the local package:

```json
{
  "dependencies": {
    "@kalamdb/client": "file:../../link/sdks/typescript/client"
  }
}
```

## Quick Start

```typescript
import { createClient, Auth } from '@kalamdb/client';

const client = createClient({
  url: 'http://localhost:2900',
  authProvider: async () => Auth.basic('admin', 'AdminPass123!'),
});

// Query data
const result = await client.query('SELECT * FROM app.users LIMIT 10');
console.log(result.results[0].rows);

// Subscribe to live changes
const unsubscribe = await client.subscribe('app.messages', (event) => {
  if (event.type === 'change') {
    console.log('New data:', event.rows);
  }
});

// Later: cleanup
await unsubscribe();
await client.disconnect();
```

## React Live Queries

`@kalamdb/react` provides `KalamProvider`, `LiveQuery`, `LiveQueries`, `useLiveQuery`, `useLiveQueries`, and `useLiveSelection`. It supports raw SQL mode and typed Drizzle mode through `@kalamdb/orm`.

```tsx
import { KalamProvider, LiveQueries, useLiveSelection } from '@kalamdb/react';
import { asc, eq } from 'drizzle-orm';
import { createClient, Auth } from '@kalamdb/client';
import { approvals, messages, toolCalls, typing } from './schema.generated';

const client = createClient({
  url: 'http://localhost:2900',
  authProvider: async () => Auth.basic('admin', 'AdminPass123!'),
});

export function AssistantScreen({ conversationId }: { conversationId: string }) {
  return (
    <KalamProvider client={client}>
      <LiveQueries
        queries={{
          messages: {
            table: messages,
            where: (table) => eq(table.conversationId, conversationId),
            orderBy: (table) => asc(table.createdAt),
            deps: [conversationId],
          },
          typing: { table: typing, where: (table) => eq(table.conversationId, conversationId), deps: [conversationId] },
          toolCalls: { table: toolCalls, where: (table) => eq(table.conversationId, conversationId), deps: [conversationId] },
          approvals: { table: approvals, where: (table) => eq(table.conversationId, conversationId), deps: [conversationId] },
        }}
      >
        {(live) => <AssistantBody live={live} />}
      </LiveQueries>
    </KalamProvider>
  );
}

function AssistantBody({ live }) {
  const assistant = useLiveSelection(live, (context) => ({
    messages: context.messages.rows,
    typingUsers: context.typing.rows.map((row) => row.userName),
    activeTools: context.toolCalls.rows.filter((row) => row.status !== 'completed'),
    pendingApprovals: context.approvals.rows.filter((row) => row.status === 'pending'),
    approve: (approvalId: string) => context.update(approvals, approvalId).set({ status: 'approved' }),
  }));

  return <AssistantLayout {...assistant} busy={live.state.loading || live.state.updating} />;
}
```

The repo includes [../../examples/react-ai-chat](../../examples/react-ai-chat), a runnable React validation app with conversation sidebar, history loading, multi-file messages, typing, tool activity, streamed replies, edit/cancel actions, and human approvals.

## API Reference

### Creating a Client

```typescript
import { createClient, Auth, type AuthProvider } from '@kalamdb/client';

const authProvider: AuthProvider = async () => Auth.basic('admin', 'AdminPass123!');

const client = createClient({
  url: 'http://localhost:2900',
  authProvider,
});
```

`createClient({ url, authProvider })` is the current high-level entrypoint. Older constructor-based examples are no longer accurate for the published SDK.

### Connection Management

```typescript
// createClient() clients do not expose a public high-level connect() call.
// HTTP queries run immediately, and the shared WebSocket opens lazily on the
// first realtime call unless wsLazyConnect is disabled.
await client.query('SELECT 1');

const unsubscribe = await client.subscribe('app.messages', handleEvent);
await unsubscribe();

// Disconnect closes the shared WebSocket and cleans up subscriptions.
await client.disconnect();
```

### SQL Queries

Execute any SQL statement - SELECT, INSERT, UPDATE, DELETE, CREATE, DROP, etc.

```typescript
// SELECT query
const result = await client.query('SELECT * FROM app.users WHERE active = true');
console.log(result.results[0].rows);

// INSERT
await client.query(`
  INSERT INTO app.users (id, name, email)
  VALUES (1, 'Alice', 'alice@example.com')
`);

// UPDATE
await client.query(`
  UPDATE app.users SET active = false WHERE id = 1
`);

// DELETE
await client.query('DELETE FROM app.users WHERE id = 1');

// DDL
await client.query(`
  CREATE TABLE app.products (
    id BIGINT PRIMARY KEY,
    name TEXT NOT NULL,
    price DOUBLE
  )
`);
```

#### Query Response Structure

```typescript
interface QueryResponse {
  status: 'success' | 'error';
  results: QueryResult[];
  took?: number;  // Execution time in milliseconds
  error?: ErrorDetail;
}

interface SchemaField {
  name: string;        // Column name
  data_type: string;   // e.g., 'BigInt', 'Text', 'Timestamp'
  index: number;       // Column index in rows array
}

interface QueryResult {
  schema: SchemaField[];  // Column definitions
  rows?: unknown[][];     // Array of row arrays (values ordered by schema index)
  row_count: number;
  message?: string;
}
```

### Convenience Methods

```typescript
// Insert data (builds INSERT statement automatically)
await client.insert('app.todos', {
  title: 'Buy groceries',
  completed: false
});

// Delete by ID
await client.delete('app.todos', '123456789');
```

## Real-Time Subscriptions

KalamDB uses a single WebSocket connection for all subscriptions. The subscription API follows modern patterns similar to Firebase and Supabase.

### Basic Subscription

```typescript
// Subscribe returns an unsubscribe function
const unsubscribe = await client.subscribe('app.messages', (event) => {
  switch (event.type) {
    case 'subscription_ack':
      console.log('Subscription confirmed');
      break;
    
    case 'initial_data_batch':
      console.log('Initial data:', event.rows);
      console.log('Batch info:', event.batch_control);
      break;
    
    case 'change':
      console.log(`${event.change_type}:`, event.rows);
      break;
    
    case 'error':
      console.error('Error:', event.message);
      break;
  }
});

// Later: stop receiving updates
await unsubscribe();
```

### Subscription with Options

Control how initial data is loaded using subscription options:

```typescript
// Subscribe with batch size option
const unsubscribe = await client.subscribe('app.messages', handleEvent, {
  batch_size: 100  // Load initial data in batches of 100 rows
});
```

### Subscribe to SQL Query

For more control, use `subscribeWithSql()` to subscribe to custom SQL queries:

```typescript
// Subscribe to filtered query
const unsubscribe = await client.subscribeWithSql(
  'SELECT * FROM chat.messages WHERE conversation_id = 1 ORDER BY created_at DESC',
  (event) => {
    if (event.type === 'change') {
      console.log('New message:', event.rows);
    }
  },
  { batch_size: 50 }
);
```

### Subscription Management

```typescript
// Get number of active subscriptions
const count = client.getSubscriptionCount();
console.log(`Active subscriptions: ${count}`);

// Get details about all subscriptions
const subscriptions = client.getSubscriptions();
for (const sub of subscriptions) {
  console.log(`ID: ${sub.id}, Table: ${sub.tableName}, Since: ${sub.createdAt}`);
}

// Check if subscribed to a specific table
if (!client.isSubscribedTo('app.messages')) {
  await client.subscribe('app.messages', handleChanges);
}

// Unsubscribe from all at once
await client.unsubscribeAll();
```

### Preventing Too Many Subscriptions

```typescript
const MAX_SUBSCRIPTIONS = 10;

async function subscribeToTable(tableName: string) {
  if (client.getSubscriptionCount() >= MAX_SUBSCRIPTIONS) {
    console.warn('Too many subscriptions! Unsubscribe from unused tables first.');
    return null;
  }
  
  return await client.subscribe(tableName, handleEvent);
}
```

### Server Message Types

```typescript
type ServerMessage =
  | {
      type: 'subscription_ack';
      subscription_id: string;
      total_rows: number;
      batch_control: BatchControl;
    }
  | {
      type: 'initial_data_batch';
      subscription_id: string;
      rows: Record<string, any>[];
      batch_control: BatchControl;
    }
  | {
      type: 'change';
      subscription_id: string;
      change_type: 'insert' | 'update' | 'delete';
      rows?: Record<string, any>[];
      old_values?: Record<string, any>[];
    }
  | {
      type: 'error';
      subscription_id: string;
      code: string;
      message: string;
    };

interface BatchControl {
  batch_num: number;
  has_more: boolean;
  status: 'loading' | 'loading_batch' | 'ready';
  last_seq_id?: string;
}
```

## Browser Usage

```html
<!DOCTYPE html>
<html>
<head>
  <title>KalamDB Example</title>
</head>
<body>
  <div id="messages"></div>
  
  <script type="module">
    import { Auth, createClient } from '/path/to/dist/index.js';
    
    const client = createClient({
      url: 'http://localhost:2900',
      authProvider: async () => Auth.basic('admin', 'AdminPass123!')
    });
    
    const unsubscribe = await client.subscribe('app.messages', (event) => {
      if (event.type === 'change' && event.rows) {
        const div = document.getElementById('messages');
        for (const row of event.rows) {
          div.innerHTML += `<p>${JSON.stringify(row)}</p>`;
        }
      }
    });
  </script>
</body>
</html>
```

## Node.js Usage

For Node.js, you need a WebSocket polyfill since Node.js doesn't have native WebSocket:

```typescript
// Install: npm install ws
import WebSocket from 'ws';

// Add to global before importing KalamDB client
(global as any).WebSocket = WebSocket;

import { createClient, Auth } from '@kalamdb/client';

const client = createClient({
  url: 'http://localhost:2900',
  authProvider: async () => Auth.basic('admin', 'AdminPass123!')
});

// ... use client
await client.disconnect();
```

## Complete Example: Chat Application

```typescript
import { createClient, Auth, ServerMessage } from '@kalamdb/client';

async function main() {
  const client = createClient({
    url: 'http://localhost:2900',
    authProvider: async () => Auth.basic('admin', 'AdminPass123!')
  });
  
  console.log('Connected to KalamDB');
  
  // Create messages table (STREAM for auto-expiring data)
  await client.query(`
    CREATE STREAM IF NOT EXISTS app.messages
    TTL_SECONDS = 86400
    (
      id BIGINT PRIMARY KEY,
      user_id TEXT,
      content TEXT,
      created_at TIMESTAMP
    )
  `);
  
  // Subscribe to new messages
  const unsubscribe = await client.subscribe('app.messages', (event: ServerMessage) => {
    if (event.type === 'change' && event.change_type === 'insert') {
      console.log('New message:', event.rows);
    }
  });
  
  console.log(`Active subscriptions: ${client.getSubscriptionCount()}`);
  
  // Insert a message
  await client.insert('app.messages', {
    id: Date.now(),
    user_id: 'alice',
    content: 'Hello, world!',
    created_at: new Date().toISOString()
  });
  
  // Keep running for 30 seconds
  await new Promise(resolve => setTimeout(resolve, 30000));
  
  // Cleanup
  await unsubscribe();
  await client.disconnect();
  console.log('Disconnected');
}

main().catch(console.error);
```

## Error Handling

```typescript
try {
  await client.query('SELECT 1');
} catch (error) {
  console.error('Initial query failed:', error);
}

try {
  const result = await client.query('SELECT * FROM nonexistent_table');
} catch (error) {
  console.error('Query failed:', error);
}

// Handle subscription errors in callback
await client.subscribe('app.data', (event) => {
  if (event.type === 'error') {
    console.error(`Subscription error [${event.code}]: ${event.message}`);
    // Optionally: reconnect or notify user
  }
});
```

## TypeScript Types

All types are exported for use in your TypeScript code:

```typescript
import {
  // Client
  KalamDBClient,
  ClientOptions,
  
  // Query types
  QueryResult,
  QueryResponse,
  ErrorDetail,
  
  // Subscription types
  ServerMessage,
  BatchControl,
  SubscriptionCallback,
  SubscriptionInfo,
  SubscriptionOptions,
  Unsubscribe
} from '@kalamdb/client';
```

## Architecture

The SDK is built on a Rust core compiled to WebAssembly:

```
┌─────────────────────────────────────┐
│     TypeScript/JavaScript API       │
│    (index.ts - type-safe wrapper)   │
└───────────────┬─────────────────────┘
                │
┌───────────────▼─────────────────────┐
│         WASM Bindings               │
│    (kalam_link.js / .wasm)          │
└───────────────┬─────────────────────┘
                │
┌───────────────▼─────────────────────┐
│         Rust Core                   │
│    (wasm.rs - WebSocket, HTTP)      │
└─────────────────────────────────────┘
```

- **Single WebSocket connection** shared by all subscriptions
- **`authProvider`-driven auth** with JWT on protected requests; `Auth.basic(user, password)` is only used for the `/v1/api/auth/login` exchange
- **WebSocket authentication** on connect
- **Subscription callbacks** stored in HashMap for efficient dispatch
