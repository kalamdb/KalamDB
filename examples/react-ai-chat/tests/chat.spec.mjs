import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('React AI chat app uses @kalamdb/react live components and workflow tables', async () => {
  const app = await readFile(new URL('../src/app/App.tsx', import.meta.url), 'utf8');
  const schema = await readFile(new URL('../src/app/schema.generated.ts', import.meta.url), 'utf8');
  const demoClient = await readFile(new URL('../src/app/demo-client.ts', import.meta.url), 'utf8');
  const client = await readFile(new URL('../src/app/client.ts', import.meta.url), 'utf8');
  const conversation = await readFile(new URL('../src/app/components/Conversation.tsx', import.meta.url), 'utf8');
  const agent = await readFile(new URL('../src/agent/index.ts', import.meta.url), 'utf8');

  assert.match(app, /<LiveQueries/);
  assert.match(app, /conversations:/);
  assert.match(app, /typingTokens:/);
  assert.match(conversation, /ChatComposer/);
  assert.match(conversation, /clientId/);
  assert.match(conversation, /approval_id/);
  assert.match(schema, /attachment: file\(["']attachment["']\)/);
  assert.match(schema, /client_id/);
  assert.match(schema, /export const approvals/);
  assert.match(schema, /export const messages/);
  assert.match(schema, /export const typing_tokens/);
  assert.match(agent, /agent_messages/);
  assert.match(agent, /agent_actions/);
  assert.match(demoClient, /localStorage/);
  assert.match(client, /namespace:\s*['"]react_ai_chat['"]/);
});

test('React AI chat example is driven by kalam dev project config', async () => {
  const kalamToml = await readFile(new URL('../kalam.toml', import.meta.url), 'utf8');
  const initialMigration = await readFile(
    new URL('../kalam/migrations/0001_init.sql', import.meta.url),
    'utf8',
  );
  const packageJson = await readFile(new URL('../package.json', import.meta.url), 'utf8');

  assert.match(kalamToml, /\[schema\]/);
  assert.match(kalamToml, /path = "chat-app\.sql"/);
  assert.match(kalamToml, /output = "src\/app\/schema\.generated\.ts"/);
  assert.match(kalamToml, /\[dev\.processes\]/);
  assert.match(kalamToml, /app = "npm run dev"/);
  assert.match(kalamToml, /agent = "npm run agent"/);
  assert.match(initialMigration, /CREATE TOPIC IF NOT EXISTS react_ai_chat\.agent_messages/);
  assert.match(initialMigration, /INSERT INTO react_ai_chat\.conversations/);
  assert.doesNotMatch(packageJson, /"setup"/);
});
