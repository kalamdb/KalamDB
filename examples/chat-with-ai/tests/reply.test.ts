import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const exampleRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

async function procedureSource(): Promise<string> {
  return readFile(resolve(exampleRoot, 'functions/src/chat_demo/on_user_message.ts'), 'utf8');
}

test('buildReply generates a contextual assistant response', async () => {
  const source = await procedureSource();
  assert.match(source, /AI reply:/);
  assert.match(source, /slowest route/);
  assert.match(source, /latency/);
});

test('buildReply handles deploy keyword', async () => {
  const source = await procedureSource();
  assert.match(source, /deploy-shaped/);
});

test('buildReply handles queue keyword', async () => {
  const source = await procedureSource();
  assert.match(source, /downstream dependency/);
});

test('buildReply uses default advice for generic messages', async () => {
  const source = await procedureSource();
  assert.match(source, /schema-first topic trigger/);
  assert.doesNotMatch(source, /runConsumer\(\)/);
});

test('assistant SHARED insert does not use EXECUTE AS', async () => {
  const source = await procedureSource();
  assert.match(source, /insert\(chatDemoMessages\)/);
  const marker = 'await ctx.orm.insert(chatDemoMessages)';
  const start = source.indexOf(marker);
  assert.ok(start >= 0, 'expected SHARED assistant insert through the invoker orm');
  const block = source.slice(start, start + 400);
  assert.doesNotMatch(block, /executeAs/);
  assert.doesNotMatch(block, /\.as\(sender\)/);
});

test('STREAM thinking events use EXECUTE AS the chatting user', async () => {
  const source = await procedureSource();
  assert.match(source, /const asSender = ctx\.orm\.as\(sender\)/);
  assert.match(source, /asSender\.insert\(chatDemoAgentEvents\)/);
});

test('on_user_message accepts a JSON-string payload from topic packing', async () => {
  const source = await procedureSource();
  assert.match(source, /function inboxPayload/);
  assert.match(source, /typeof raw === "string"/);
  assert.match(source, /JSON\.parse\(raw\)/);
});
