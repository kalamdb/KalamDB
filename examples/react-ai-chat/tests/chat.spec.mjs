import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('React AI chat app uses @kalamdb/react live components and workflow tables', async () => {
  const app = await readFile(new URL('../src/app/App.tsx', import.meta.url), 'utf8');
  const schema = await readFile(new URL('../src/app/schema.generated.ts', import.meta.url), 'utf8');
  const demoClient = await readFile(new URL('../src/app/demo-client.ts', import.meta.url), 'utf8');
  const conversation = await readFile(new URL('../src/app/components/Conversation.tsx', import.meta.url), 'utf8');
  const agent = await readFile(new URL('../src/agent/index.ts', import.meta.url), 'utf8');

  assert.match(app, /<LiveQueries/);
  assert.match(app, /conversations:/);
  assert.match(app, /typingTokens:/);
  assert.match(conversation, /ChatComposer/);
  assert.match(conversation, /clientId/);
  assert.match(conversation, /approvalActions/);
  assert.match(schema, /attachment: file\('attachment'\)/);
  assert.match(schema, /clientId/);
  assert.match(schema, /react_ai_chat\.approvals/);
  assert.match(schema, /react_ai_chat\.messages/);
  assert.match(schema, /react_ai_chat\.typing_tokens/);
  assert.match(agent, /agent_messages/);
  assert.match(agent, /agent_actions/);
  assert.match(demoClient, /localStorage/);
});