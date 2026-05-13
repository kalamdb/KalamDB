# API Contract: WASM Client Interface

**Feature**: 006-docker-wasm-examples  
**Component**: kalam-link (WASM)  
**Purpose**: Define TypeScript/JavaScript interface for WASM client

---

## Client Initialization

### Constructor

```typescript
class KalamClient {
  constructor(url: string, apiKey: string);
}
```

**Parameters**:
- `url`: KalamDB server WebSocket URL (e.g., `ws://localhost:2900`)
- `apiKey`: User's API key for authentication

**Throws**:
- `Error`: If `url` is empty or invalid
- `Error`: If `apiKey` is empty

**Example**:
```typescript
const client = new KalamClient(
  'ws://localhost:2900',
  '550e8400-e29b-41d4-a716-446655440000'
);
```

---

## Connection Management

### connect()

```typescript
async connect(): Promise<void>
```

**Purpose**: Establish WebSocket connection to KalamDB server

**Returns**: Promise that resolves when connected

**Throws**:
- `Error`: If connection fails (invalid URL, network error, auth failure)

**Example**:
```typescript
await client.connect();
console.log('Connected to KalamDB');
```

### disconnect()

```typescript
disconnect(): void
```

**Purpose**: Close WebSocket connection

**Example**:
```typescript
client.disconnect();
```

### isConnected()

```typescript
isConnected(): boolean
```

**Returns**: `true` if WebSocket is connected, `false` otherwise

---

## Data Operations

### insert()

```typescript
async insert(table: string, data: Record<string, any>): Promise<number>
```

**Parameters**:
- `table`: Table name
- `data`: Key-value pairs of column data

**Returns**: Promise resolving to inserted row ID

**Throws**:
- `Error`: If not connected
- `Error`: If insert fails (validation, constraint violation)

**Example**:
```typescript
const id = await client.insert('todos', {
  title: 'Buy groceries',
  completed: false
});
console.log(`Inserted TODO with ID: ${id}`);
```

### delete()

```typescript
async delete(table: string, id: number): Promise<void>
```

**Parameters**:
- `table`: Table name
- `id`: Row ID to delete

**Returns**: Promise resolving when delete completes (soft delete)

**Throws**:
- `Error`: If not connected
- `Error`: If row not found

**Example**:
```typescript
await client.delete('todos', 5);
console.log('TODO deleted (soft delete)');
```

### query()

```typescript
async query(sql: string): Promise<QueryResult>
```

**Parameters**:
- `sql`: SQL SELECT statement

**Returns**: Promise resolving to query results

**Type Definitions**:
```typescript
interface QueryResult {
  columns: string[];
  rows: any[][];
  rowCount: number;
}
```

**Throws**:
- `Error`: If not connected
- `Error`: If SQL is invalid

**Example**:
```typescript
const result = await client.query('SELECT * FROM todos WHERE completed = false');
console.log(`Found ${result.rowCount} incomplete todos`);
result.rows.forEach(row => {
  console.log(`- ${row[1]}`); // title column
});
```

---

## Subscriptions

### subscribe()

```typescript
subscribe(
  table: string,
  callback: (event: SubscriptionEvent) => void,
  options?: SubscribeOptions
): void
```

**Parameters**:
- `table`: Table name to subscribe to
- `callback`: Function called for each event
- `options`: Optional subscription configuration

**Type Definitions**:
```typescript
interface SubscriptionEvent {
  type: 'insert' | 'update' | 'delete';
  table: string;
  id: number;
  data?: Record<string, any>; // Present for insert/update
}

interface SubscribeOptions {
  fromId?: number; // Only receive events for IDs >= fromId
}
```

**Example**:
```typescript
// Subscribe to all changes
client.subscribe('todos', (event) => {
  console.log(`Event: ${event.type} on row ${event.id}`);
  if (event.type === 'insert') {
    console.log('New TODO:', event.data);
  }
});

// Subscribe from specific ID (for sync)
const lastKnownId = parseInt(localStorage.getItem('lastTodoId') || '0');
client.subscribe('todos', handleEvent, { fromId: lastKnownId });
```

### unsubscribe()

```typescript
unsubscribe(table: string): void
```

**Parameters**:
- `table`: Table name to unsubscribe from

**Example**:
```typescript
client.unsubscribe('todos');
```

---

## Error Handling

### Error Types

```typescript
// Connection errors
throw new Error('Failed to connect: Connection refused');
throw new Error('Authentication failed: Invalid API key');

// Operation errors
throw new Error('Not connected: Call connect() first');
throw new Error('Insert failed: Title cannot be empty');
throw new Error('Delete failed: Row not found');

// Query errors
throw new Error('Query failed: Invalid SQL syntax');
```

### Example Error Handling

```typescript
try {
  await client.connect();
} catch (error) {
  console.error('Connection failed:', error.message);
  // Show error to user
}

try {
  await client.insert('todos', { title: '' });
} catch (error) {
  console.error('Insert failed:', error.message);
  // Validation error
}
```

---

## Complete Example

```typescript
import { KalamClient } from 'kalam-link';

async function setupTodoApp() {
  // Initialize client
  const client = new KalamClient(
    'ws://localhost:2900',
    process.env.KALAMDB_API_KEY!
  );

  try {
    // Connect
    await client.connect();
    console.log('Connected!');

    // Subscribe to changes
    client.subscribe('todos', (event) => {
      if (event.type === 'insert') {
        addTodoToUI(event.data!);
      } else if (event.type === 'delete') {
        removeTodoFromUI(event.id);
      }
    });

    // Load initial data
    const result = await client.query('SELECT * FROM todos ORDER BY id');
    result.rows.forEach(row => {
      addTodoToUI({
        id: row[0],
        title: row[1],
        completed: row[2],
        created_at: row[3]
      });
    });

    // Add new TODO
    const newId = await client.insert('todos', {
      title: 'Test subscription',
      completed: false
    });
    console.log(`Inserted TODO ${newId}`);

  } catch (error) {
    console.error('Error:', error);
  }
}

function addTodoToUI(todo: any) {
  // Update React state, etc.
}

function removeTodoFromUI(id: number) {
  // Update React state, etc.
}
```

---

## TypeScript Type Definitions

Full type definition file (`kalam-link.d.ts`):

```typescript
declare module 'kalam-link' {
  export class KalamClient {
    constructor(url: string, apiKey: string);
    
    connect(): Promise<void>;
    disconnect(): void;
    isConnected(): boolean;
    
    insert(table: string, data: Record<string, any>): Promise<number>;
    delete(table: string, id: number): Promise<void>;
    query(sql: string): Promise<QueryResult>;
    
    subscribe(
      table: string,
      callback: (event: SubscriptionEvent) => void,
      options?: SubscribeOptions
    ): void;
    unsubscribe(table: string): void;
  }

  export interface QueryResult {
    columns: string[];
    rows: any[][];
    rowCount: number;
  }

  export interface SubscriptionEvent {
    type: 'insert' | 'update' | 'delete';
    table: string;
    id: number;
    data?: Record<string, any>;
  }

  export interface SubscribeOptions {
    fromId?: number;
  }
}
```

---

## Browser Compatibility

**Minimum Requirements**:
- Chrome 57+
- Firefox 52+
- Safari 11+
- Edge 16+

**Required Features**:
- WebAssembly support
- WebSocket API
- ES2015 (ES6) JavaScript
- localStorage API

**Polyfills**: Not required for supported browsers

---

## Size and Performance

**Bundle Size** (estimated):
- WASM module: ~200KB (gzipped)
- JavaScript bindings: ~50KB (gzipped)
- Total: ~250KB

**Performance**:
- Connection establishment: <500ms
- Insert/delete operations: <50ms
- Query execution: <100ms (depending on result size)
- Subscription events: <10ms latency from server

---

## Security

1. **API Key Handling**: Never log or expose API key in client-side code
2. **Transport**: Use WSS (WebSocket Secure) in production
3. **Origin Validation**: Server should validate WebSocket origin
4. **CORS**: Configure server CORS policy appropriately
