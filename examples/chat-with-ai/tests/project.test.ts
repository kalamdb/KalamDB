import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const exampleRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

async function readExample(relativePath: string): Promise<string> {
  return readFile(resolve(exampleRoot, relativePath), 'utf8');
}

test('chat-with-ai is a kalam CLI project with generated TypeScript in src/generated', async () => {
  const kalamToml = await readExample('kalam.toml');
  const packageJson = await readExample('package.json');
  const schemaSql = await readExample('kalam/schema.sql');
  const app = await readExample('src/App.tsx');
  const agent = await readExample('src/agent.ts');
  const generated = await readExample('src/generated/kalam.ts');

  assert.match(kalamToml, /path = "kalam\/schema\.sql"/);
  assert.match(kalamToml, /output = "src\/generated\/kalam\.ts"/);
  assert.match(kalamToml, /generate_types = true/);
  assert.match(kalamToml, /app = "npm run dev"/);
  assert.match(kalamToml, /agent = "npm run agent"/);

  assert.doesNotMatch(packageJson, /"setup"/);
  assert.doesNotMatch(packageJson, /generate:schema/);
  assert.equal(existsSync(resolve(exampleRoot, 'setup.mjs')), false);
  assert.equal(existsSync(resolve(exampleRoot, 'scripts/generate-schema.mjs')), false);
  assert.equal(existsSync(resolve(exampleRoot, 'src/schema.generated.ts')), false);

  assert.match(schemaSql, /CREATE SHARED TABLE IF NOT EXISTS chat_demo\.room_members/);
  assert.match(schemaSql, /id TEXT PRIMARY KEY/);
  assert.doesNotMatch(schemaSql, /PRIMARY KEY \(user_id, room_id\)/);
  assert.match(schemaSql, /CREATE POLICY messages_member_select/);
  assert.match(schemaSql, /CREATE STREAM TABLE IF NOT EXISTS chat_demo\.agent_events/);
  assert.match(schemaSql, /ALTER TOPIC chat_demo\.ai_inbox ADD SOURCE chat_demo\.messages ON INSERT/);

  assert.match(app, /from '\.\/generated\/kalam'/);
  assert.match(app, /liveTable/);
  assert.match(agent, /from '\.\/generated\/kalam\.js'/);
  assert.match(agent, /runConsumer/);
  assert.match(generated, /export const chat_demo_messages = kTable\.shared/);
  assert.match(generated, /export const chat_demo_agent_events = kTable\.stream/);
});

function sliceBetween(source: string, start: string, end: string): string {
  const startIndex = source.indexOf(start);
  const endIndex = source.indexOf(end, startIndex + start.length);
  assert.ok(startIndex >= 0, `expected to find ${JSON.stringify(start)}`);
  assert.ok(endIndex > startIndex, `expected to find ${JSON.stringify(end)} after start marker`);
  return source.slice(startIndex, endIndex);
}

test('agent inserts SHARED assistant replies without EXECUTE AS USER', async () => {
  const agent = await readExample('src/agent.ts');
  const insertBlock = sliceBetween(
    agent,
    'const insertAssistantMessage',
    'console.log(`[chat-demo-agent] starting',
  );

  assert.match(insertBlock, /db\.insert\(chatMessages\)/);
  assert.doesNotMatch(insertBlock, /executeAsUser/);
});

test('agent still uses EXECUTE AS USER for STREAM thinking events', async () => {
  const agent = await readExample('src/agent.ts');
  const emitBlock = sliceBetween(agent, 'const emitEvent', 'const insertAssistantMessage');

  assert.match(emitBlock, /executeAsUser/);
  assert.match(emitBlock, /db\.insert\(agentEvents\)/);
});

test('app looks up membership and the room before inserting seed rows', async () => {
  const app = await readExample('src/App.tsx');
  const joinBlock = sliceBetween(app, 'async function ensureRoomAccess', 'export function App');

  assert.match(joinBlock, /\.from\(roomMembers\)/);
  assert.match(joinBlock, /\.from\(rooms\)/);
  assert.match(joinBlock, /existingMembership\.length === 0/);
  assert.match(joinBlock, /existingRoom\.length === 0/);
  assert.doesNotMatch(joinBlock, /try \{/);
});

test('playwright helpers insert SHARED messages as DBA instead of EXECUTE AS USER', async () => {
  const spec = await readExample('tests/chat.spec.mjs');
  const insertUser = sliceBetween(spec, 'async function insertUserMessage', 'function sqlLiteral');
  const insertAssistant = sliceBetween(
    spec,
    'async function insertAssistantMessages',
    'async function seedChatHistory',
  );

  assert.match(insertUser, /INSERT INTO chat_demo\.messages/);
  assert.match(insertAssistant, /INSERT INTO chat_demo\.messages/);
  assert.doesNotMatch(insertUser, /EXECUTE AS USER \$\{/);
  assert.doesNotMatch(insertAssistant, /EXECUTE AS USER \$\{/);
});

test('schema.sql keeps teaching comments immediately before CREATE SHARED TABLE', async () => {
  const schemaSql = await readExample('kalam/schema.sql');
  assert.match(
    schemaSql,
    /-- Rooms everyone can create[^\n]*\nCREATE SHARED TABLE IF NOT EXISTS chat_demo\.rooms/,
  );
});
