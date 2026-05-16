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

test("after an approved request_approval, mock emits delete_conversation with the right id", async () => {
  const events = await collect([
    { role: "system", content: "conversation_id = 11111111-2222-3333-4444-555555555555" },
    { role: "user", content: "Please delete this conversation." },
    {
      role: "assistant",
      content: "I can do that — getting your approval first. ",
      toolCalls: [{ id: "mock_x", name: "request_approval", arguments: { question: "Approve?" } }],
    },
    { role: "tool", toolCallId: "mock_x", content: "approved" },
  ]);
  const toolEvent = events.find((e) => e.type === "tool_call") as
    | { type: "tool_call"; call: { name: string; arguments: { conversation_id: string } } }
    | undefined;
  assert.ok(toolEvent);
  assert.equal(toolEvent.call.name, "delete_conversation");
  assert.equal(toolEvent.call.arguments.conversation_id, "11111111-2222-3333-4444-555555555555");
});

test("after delete_conversation succeeds, mock wraps up with a confirmation", async () => {
  const events = await collect([
    { role: "user", content: "Delete my account." },
    {
      role: "assistant",
      content: "",
      toolCalls: [
        {
          id: "mock_d",
          name: "delete_conversation",
          arguments: { conversation_id: "11111111-2222-3333-4444-555555555555" },
        },
      ],
    },
    {
      role: "tool",
      toolCallId: "mock_d",
      content: "deleted conversation 11111111-... and all related rows",
    },
  ]);
  const text = events
    .filter((e) => e.type === "text")
    .map((e) => (e as { delta: string }).delta)
    .join("");
  assert.match(text, /deleted/i);
});

test("fuzzy 'what is' prompt triggers a search_documents tool call (RAG)", async () => {
  const events = await collect([
    { role: "user", content: "What is a KalamDB topic and how does it work?" },
  ]);
  const toolEvent = events.find((e) => e.type === "tool_call") as
    | { type: "tool_call"; call: { name: string; arguments: { query: string; limit?: number } } }
    | undefined;
  assert.ok(toolEvent);
  assert.equal(toolEvent.call.name, "search_documents");
  assert.match(toolEvent.call.arguments.query, /topic/);
});

test("after search_documents returns rows, mock phrases the answer with a citation", async () => {
  const events = await collect([
    { role: "user", content: "What is a topic?" },
    {
      role: "assistant",
      content: "",
      toolCalls: [
        { id: "mock_s", name: "search_documents", arguments: { query: "what is a topic" } },
      ],
    },
    {
      role: "tool",
      toolCallId: "mock_s",
      content:
        '{"row_count":1,"rows":[{"id":"doc-topics","title":"Topics & runConsumer","body":"Topics are an append-only stream of events.","distance":0.05}]}',
    },
  ]);
  const text = events
    .filter((e) => e.type === "text")
    .map((e) => (e as { delta: string }).delta)
    .join("");
  assert.match(text, /source: "Topics & runConsumer"/);
});

test("query-shaped prompt triggers a query_database tool call", async () => {
  const events = await collect([{ role: "user", content: "How many messages do I have?" }]);
  const toolEvent = events.find((e) => e.type === "tool_call") as
    | { type: "tool_call"; call: { name: string; arguments: { sql: string } } }
    | undefined;
  assert.ok(toolEvent);
  assert.equal(toolEvent.call.name, "query_database");
  assert.match(toolEvent.call.arguments.sql, /SELECT count\(\*\) AS n FROM chat\.messages/);
});

test("after query_database returns rows, mock phrases the count", async () => {
  const events = await collect([
    { role: "user", content: "How many conversations do I have?" },
    {
      role: "assistant",
      content: "",
      toolCalls: [
        {
          id: "mock_q",
          name: "query_database",
          arguments: { sql: "SELECT count(*) AS n FROM chat.conversations" },
        },
      ],
    },
    { role: "tool", toolCallId: "mock_q", content: '{"row_count":1,"rows":[{"n":7}]}' },
  ]);
  const text = events
    .filter((e) => e.type === "text")
    .map((e) => (e as { delta: string }).delta)
    .join("");
  assert.match(text, /\b7\b/);
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
