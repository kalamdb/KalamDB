import assert from 'node:assert/strict';
import test from 'node:test';

import { runConsumer } from '../dist/src/index.js';
import { createConsumerScenarioClient, makeConsumerMessage } from './use-case-helpers.mjs';

test('consumer use case 03: AI chat summarizer uses the LLM context and persists summaries', async () => {
  const message = makeConsumerMessage({
    offset: 20,
    topic: 'ai_chat.turn_events',
    group_id: 'ai-summary-worker',
    user: 'user-7',
    op: 'Insert',
    payload: {
      id: 'turn-20',
      conversation_id: 'conv-7',
      role: 'user',
      body: 'Please compare the deployment risks and propose a rollback plan.',
      token_count: 1880,
      _table: 'ai_chat.turns',
    },
  });
  const { client, state } = createConsumerScenarioClient([message]);
  const llmInputs = [];

  await runConsumer({
    client,
    name: 'ai-summary-worker',
    topic: 'ai_chat.turn_events',
    groupId: 'ai-summary-worker',
    systemPrompt: 'Summarize chat turns for fast conversation recall.',
    llm: {
      complete: async (input) => {
        llmInputs.push(input);
        return 'deployment risks with rollback plan';
      },
    },
    retry: { maxAttempts: 1, initialBackoffMs: 0, maxBackoffMs: 0 },
    onChange: async (ctx, change) => {
      if (change.data.token_count < 1000) return;
      const summary = await ctx.llm.complete({
        messages: [{ role: 'user', content: String(change.data.body) }],
      });
      await ctx.sql(
        'INSERT INTO ai_chat.conversation_summaries (conversation_id, turn_id, summary) VALUES ($1, $2, $3)',
        [change.data.conversation_id, change.data.id, summary],
      );
    },
  });

  assert.equal(llmInputs.length, 1);
  assert.equal(llmInputs[0].systemPrompt, 'Summarize chat turns for fast conversation recall.');
  assert.equal(llmInputs[0].runKey, 'ai-summary-worker:ai_chat.turn_events:0:20');
  assert.equal(llmInputs[0].change.conversation_id, 'conv-7');
  assert.deepEqual(state.queryCalls[0].params, ['conv-7', 'turn-20', 'deployment risks with rollback plan']);
  assert.deepEqual(state.ackedOffsets, [20]);
});