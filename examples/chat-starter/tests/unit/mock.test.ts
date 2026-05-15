import { test } from "node:test";
import assert from "node:assert/strict";
import { MockAdapter } from "../../src/lib/llm/mock.js";
import type { LlmMessage, LlmStreamEvent } from "../../src/lib/llm/index.js";

async function collect(
  messages: LlmMessage[],
  options?: { abortAfter?: number },
): Promise<LlmStreamEvent[]> {
  const adapter = new MockAdapter();
  const controller = new AbortController();
  const out: LlmStreamEvent[] = [];
  const stream = adapter.stream({ messages, signal: controller.signal });
  for await (const event of stream) {
    out.push(event);
    if (options?.abortAfter !== undefined && out.length >= options.abortAfter) {
      controller.abort();
    }
  }
  return out;
}

test("plain greeting streams text then done:stop", async () => {
  const events = await collect([{ role: "user", content: "hi" }]);
  assert.equal(events.at(-1)?.type, "done");
  assert.equal((events.at(-1) as { reason: string }).reason, "stop");
  const text = events
    .filter((e) => e.type === "text")
    .map((e) => (e as { delta: string }).delta)
    .join("");
  assert.match(text, /mock/i);
});

test("destructive verb triggers a request_approval tool call (with text preface)", async () => {
  const events = await collect([
    { role: "user", content: "Please delete my old account data immediately." },
  ]);
  const types = events.map((e) => e.type);
  // We expect: at least one text delta (preface), then a tool_call, then done.
  assert.ok(types.includes("text"), "preface text should be emitted");
  const toolEvent = events.find((e) => e.type === "tool_call");
  assert.ok(toolEvent, "tool_call should be emitted");
  assert.equal((toolEvent as { call: { name: string } }).call.name, "request_approval");
  assert.equal(events.at(-1)?.type, "done");
  assert.equal((events.at(-1) as { reason: string }).reason, "tool_calls");
});

test("resuming after an approved tool result yields a confirmation reply", async () => {
  const events = await collect([
    { role: "user", content: "Please delete my old account data immediately." },
    {
      role: "assistant",
      content: "Let me get explicit approval before I do that.",
      toolCalls: [{ id: "mock_x", name: "request_approval", arguments: { question: "Approve?" } }],
    },
    { role: "tool", toolCallId: "mock_x", content: "approved" },
  ]);
  const text = events
    .filter((e) => e.type === "text")
    .map((e) => (e as { delta: string }).delta)
    .join("");
  assert.match(text, /approved/i);
  assert.equal(events.at(-1)?.type, "done");
});

test("__slow_stream__ produces many chunks (slow path)", async () => {
  // Abort after a few chunks to keep the test fast.
  const events = await collect(
    [{ role: "user", content: "__slow_stream__ please write a long response" }],
    { abortAfter: 5 },
  );
  // We expect at least the preface + a few chunk_ deltas, no done frame
  // (since we aborted mid-stream).
  const textDeltas = events.filter((e) => e.type === "text") as Array<{ delta: string }>;
  assert.ok(textDeltas.length >= 3, "should have streamed at least 3 chunks before abort");
  assert.match(
    textDeltas.map((d) => d.delta).join(""),
    /chunk_/,
    "slow path should emit chunk_NNN tokens",
  );
});

test("abort signal stops the generator before completion", async () => {
  const adapter = new MockAdapter();
  const controller = new AbortController();
  const messages: LlmMessage[] = [{ role: "user", content: "hi" }];
  const stream = adapter.stream({ messages, signal: controller.signal });
  let count = 0;
  for await (const _ev of stream) {
    count++;
    controller.abort();
  }
  // Generator should exit; we should not loop forever.
  assert.ok(count >= 1);
});
