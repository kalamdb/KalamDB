/**
 * Integration tests for generateSchema after live DDL changes.
 *
 * Each scenario issues real DDL through a test client, then re-calls
 * generateSchema and asserts that the output reflects the mutated schema.
 * This covers the production workflow: "migrate the database, run the
 * code-generator, get an updated schema.ts".
 *
 * Requires a running KalamDB server and KALAMDB_TEST_PASSWORD set.
 */

import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { generateSchema } from '../dist/index.js';
import { requirePassword, createTestClient } from './helpers.mjs';

requirePassword();

const NS = 'test_gen_chg';

let client;

before(async () => {
  client = createTestClient();
  await client.initialize();
  await client.query(`CREATE NAMESPACE IF NOT EXISTS ${NS}`);
});

after(async () => {
  // Best-effort cleanup — individual tests clean up their own tables.
  await client?.query(`DROP NAMESPACE IF EXISTS ${NS}`).catch(() => {});
  await client?.disconnect();
});

// ---------------------------------------------------------------------------
// Scenario 1 — New column added: second schema includes the column, first did not
// ---------------------------------------------------------------------------
describe('generateSchema reflects column addition', () => {
  before(async () => {
    await client.query(`DROP TABLE IF EXISTS ${NS}.events`);
    await client.query(`CREATE TABLE ${NS}.events (id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(), title TEXT NOT NULL)`);
  });

  after(async () => {
    await client.query(`DROP TABLE IF EXISTS ${NS}.events`);
  });

  it('schema before adding column has no summary field', async () => {
    const schema = await generateSchema(client, { namespaces: [NS] });
    const block = extractTableBlock(schema, `${NS}_events`);
    assert.ok(block !== null, 'table block must exist');
    assert.ok(!block.includes('summary'), 'summary column must not exist yet');
    assert.ok(block.includes('title:'), 'title column must exist');
  });

  it('schema after adding column includes the new field', async () => {
    // Re-create table with additional column (KalamDB uses DROP+CREATE for schema changes).
    await client.query(`DROP TABLE IF EXISTS ${NS}.events`);
    await client.query(`CREATE TABLE ${NS}.events (id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(), title TEXT NOT NULL, summary TEXT)`);

    const schema = await generateSchema(client, { namespaces: [NS] });
    const block = extractTableBlock(schema, `${NS}_events`);
    assert.ok(block !== null, 'table block must exist after column addition');
    assert.ok(block.includes('title:'), 'title column must still be present');
    assert.ok(block.includes('summary:'), 'newly added summary column must appear');
  });
});

// ---------------------------------------------------------------------------
// Scenario 2 — Table dropped: second schema does not contain it
// ---------------------------------------------------------------------------
describe('generateSchema reflects table drop', () => {
  before(async () => {
    await client.query(`DROP TABLE IF EXISTS ${NS}.temp_sessions`);
    await client.query(`CREATE TABLE ${NS}.temp_sessions (session_id TEXT PRIMARY KEY, user_id TEXT NOT NULL)`);
  });

  after(async () => {
    await client.query(`DROP TABLE IF EXISTS ${NS}.temp_sessions`).catch(() => {});
  });

  it('schema before dropping table contains the table', async () => {
    const schema = await generateSchema(client, { namespaces: [NS] });
    assert.ok(schema.includes(`${NS}_temp_sessions`), 'temp_sessions must appear before drop');
  });

  it('schema after dropping table no longer contains it', async () => {
    await client.query(`DROP TABLE IF EXISTS ${NS}.temp_sessions`);

    const schema = await generateSchema(client, { namespaces: [NS] });
    assert.ok(!schema.includes(`${NS}_temp_sessions`), 'temp_sessions must be absent after drop');
  });
});

// ---------------------------------------------------------------------------
// Scenario 3 — New table created: second schema picks it up
// ---------------------------------------------------------------------------
describe('generateSchema picks up newly created table', () => {
  after(async () => {
    await client.query(`DROP TABLE IF EXISTS ${NS}.notifications`).catch(() => {});
  });

  it('schema before creating table has no notifications table', async () => {
    await client.query(`DROP TABLE IF EXISTS ${NS}.notifications`);

    const schema = await generateSchema(client, { namespaces: [NS] });
    assert.ok(!schema.includes(`${NS}_notifications`), 'notifications must not exist yet');
  });

  it('schema after creating table includes the notifications table', async () => {
    await client.query(`CREATE TABLE ${NS}.notifications (id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(), message TEXT NOT NULL, read BOOLEAN NOT NULL DEFAULT FALSE)`);

    const schema = await generateSchema(client, { namespaces: [NS] });
    const block = extractTableBlock(schema, `${NS}_notifications`);
    assert.ok(block !== null, 'notifications table block must appear');
    assert.ok(block.includes('message:'), 'message column must be present');
    assert.ok(block.includes('read:'), 'read column must be present');
  });
});

// ---------------------------------------------------------------------------
// Scenario 4 — Schema idempotency: two consecutive calls produce identical output
// ---------------------------------------------------------------------------
describe('generateSchema is idempotent across consecutive calls', () => {
  before(async () => {
    await client.query(`DROP TABLE IF EXISTS ${NS}.stable`);
    await client.query(`CREATE TABLE ${NS}.stable (id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(), value TEXT NOT NULL)`);
  });

  after(async () => {
    await client.query(`DROP TABLE IF EXISTS ${NS}.stable`);
  });

  it('two consecutive generates with no DDL change produce identical output', async () => {
    const opts = { namespaces: [NS] };
    const first = await generateSchema(client, opts);
    const second = await generateSchema(client, opts);

    assert.equal(first, second, 'consecutive schema outputs must be identical (deterministic ordering)');
  });
});

// ---------------------------------------------------------------------------
// Scenario 5 — Column nullability change: notNull() modifier is updated
// ---------------------------------------------------------------------------
describe('generateSchema reflects column nullability change', () => {
  before(async () => {
    await client.query(`DROP TABLE IF EXISTS ${NS}.products`);
    // description is nullable to start
    await client.query(`CREATE TABLE ${NS}.products (id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(), name TEXT NOT NULL, description TEXT)`);
  });

  after(async () => {
    await client.query(`DROP TABLE IF EXISTS ${NS}.products`).catch(() => {});
  });

  it('nullable column is generated without notNull()', async () => {
    const schema = await generateSchema(client, { namespaces: [NS] });
    const block = extractTableBlock(schema, `${NS}_products`);
    assert.ok(block !== null, 'products block must exist');

    const descLine = extractColumnLine(block, 'description');
    assert.ok(descLine !== null, 'description line must exist');
    assert.ok(!descLine.includes('.notNull()'), 'nullable column must not have notNull()');
  });

  it('after recreating with NOT NULL the column gets notNull()', async () => {
    // Recreate with description as NOT NULL
    await client.query(`DROP TABLE IF EXISTS ${NS}.products`);
    await client.query(`CREATE TABLE ${NS}.products (id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(), name TEXT NOT NULL, description TEXT NOT NULL)`);

    const schema = await generateSchema(client, { namespaces: [NS] });
    const block = extractTableBlock(schema, `${NS}_products`);
    assert.ok(block !== null, 'products block must exist after recreate');

    const descLine = extractColumnLine(block, 'description');
    assert.ok(descLine !== null, 'description line must still exist');
    assert.ok(descLine.includes('.notNull()'), 'now-required column must have notNull()');
  });
});

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Extracts the kTable block for a given export identifier from a generated schema.
 * Returns the text from `export const <id> = kTable...` through the closing `});`.
 */
function extractTableBlock(schema, exportId) {
  const lines = schema.split('\n');
  const startIdx = lines.findIndex((line) => line.startsWith(`export const ${exportId} = kTable`));
  if (startIdx === -1) return null;

  let depth = 0;
  let endIdx = startIdx;
  for (let i = startIdx; i < lines.length; i++) {
    for (const ch of lines[i]) {
      if (ch === '(' || ch === '{') depth += 1;
      else if (ch === ')' || ch === '}') depth -= 1;
    }
    endIdx = i;
    if (depth <= 0) break;
  }

  return lines.slice(startIdx, endIdx + 1).join('\n');
}

/**
 * Returns the column definition line for `colName:` within a table block.
 */
function extractColumnLine(block, colName) {
  const line = block.split('\n').find((l) => l.trim().startsWith(`${colName}:`));
  return line ?? null;
}
