import { test } from "node:test";
import assert from "node:assert/strict";
import { mapStopReason, splitSystemAndMessages } from "../../src/lib/llm/anthropic.js";
import type { LlmMessage } from "../../src/lib/llm/index.js";

test("mapStopReason maps Anthropic stop reasons to LlmStreamEvent reasons", () => {
  assert.equal(mapStopReason("end_turn"), "stop");
  assert.equal(mapStopReason("stop_sequence"), "stop");
  assert.equal(mapStopReason("tool_use"), "tool_calls");
  assert.equal(mapStopReason("max_tokens"), "length");
  assert.equal(mapStopReason("refusal"), "error");
  assert.equal(mapStopReason("anything_else"), "error");
});

test("splitSystemAndMessages collapses multiple system messages", () => {
  const messages: LlmMessage[] = [
    { role: "system", content: "First system." },
    { role: "system", content: "Second system." },
    { role: "user", content: "Hello." },
  ];
  const out = splitSystemAndMessages(messages);
  assert.equal(out.system, "First system.\n\nSecond system.");
  assert.equal(out.messages.length, 1);
  assert.deepEqual(out.messages[0], { role: "user", content: "Hello." });
});

test("splitSystemAndMessages returns undefined system when none present", () => {
  const out = splitSystemAndMessages([{ role: "user", content: "Hi" }]);
  assert.equal(out.system, undefined);
});

test("splitSystemAndMessages translates assistant tool_calls into Anthropic tool_use blocks", () => {
  const messages: LlmMessage[] = [
    {
      role: "assistant",
      content: "Let me check.",
      toolCalls: [{ id: "call_1", name: "request_approval", arguments: { question: "Approve?" } }],
    },
  ];
  const out = splitSystemAndMessages(messages);
  const assistant = out.messages[0];
  assert.equal(assistant?.role, "assistant");
  assert.ok(Array.isArray(assistant.content));
  const blocks = assistant.content as Array<{ type: string }>;
  assert.equal(blocks[0]?.type, "text");
  assert.equal(blocks[1]?.type, "tool_use");
});

test("splitSystemAndMessages emits an empty-string assistant when no content + no tool_calls", () => {
  const messages: LlmMessage[] = [{ role: "assistant", content: "" }];
  const out = splitSystemAndMessages(messages);
  assert.equal(out.messages[0]?.content, "");
});

test("splitSystemAndMessages converts a tool message into a user tool_result", () => {
  const messages: LlmMessage[] = [{ role: "tool", toolCallId: "call_1", content: "approved" }];
  const out = splitSystemAndMessages(messages);
  assert.equal(out.messages[0]?.role, "user");
  const blocks = out.messages[0]?.content as Array<{
    type: string;
    tool_use_id: string;
    content: string;
  }>;
  assert.equal(blocks[0]?.type, "tool_result");
  assert.equal(blocks[0]?.tool_use_id, "call_1");
  assert.equal(blocks[0]?.content, "approved");
});
