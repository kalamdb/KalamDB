import test from 'node:test';
import assert from 'node:assert/strict';
import { buildApprovalMessage, buildAssistantReply, createToolPlan, splitIntoTokenChunks } from '../src/agent/index';

test('buildApprovalMessage explains why the assistant paused', () => {
  const prompt = buildApprovalMessage('refund the customer');
  assert.match(prompt, /approve/i);
  assert.match(prompt, /human decision/i);
});

test('buildAssistantReply includes deployment-specific guidance', () => {
  const reply = buildAssistantReply('deploy regression');
  assert.match(reply, /release context/);
  assert.match(reply, /deploy regression/);
});

test('createToolPlan requires approval for customer-impacting requests', () => {
  const plan = createToolPlan('refund the customer after approval');
  assert.equal(plan.toolName, 'human_approval');
  assert.equal(plan.requiresApproval, true);
});

test('createToolPlan uses a normal tool for low-risk requests', () => {
  const plan = createToolPlan('check the deploy');
  assert.equal(plan.toolName, 'release_lookup');
  assert.equal(plan.requiresApproval, false);
});

test('splitIntoTokenChunks throttles streamed assistant output into chunks', () => {
  assert.deepEqual(splitIntoTokenChunks('abcdef', 2), ['ab', 'cd', 'ef']);
});