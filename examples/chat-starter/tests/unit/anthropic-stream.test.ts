import { test } from "node:test";
import assert from "node:assert/strict";
import { AnthropicAdapter } from "../../src/lib/llm/anthropic.js";
import type { LlmStreamEvent } from "../../src/lib/llm/index.js";

// Anthropic streams as `event: foo\ndata: {...}\n\n` per frame. We only
// inspect the data line, so the test wire format matches.
function sseResponse(frames: Array<Record<string, unknown>>, status = 200): Response {
  const encoder = new TextEncoder();
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const f of frames) {
        controller.enqueue(
          encoder.encode(`event: ${String(f.type)}\ndata: ${JSON.stringify(f)}\n\n`),
        );
      }
      controller.close();
    },
  });
  return new Response(body, { status, headers: { "content-type": "text/event-stream" } });
}

async function withMockedFetch<T>(resp: Response, fn: () => Promise<T>): Promise<T> {
  const real = globalThis.fetch;
  globalThis.fetch = (async () => resp) as typeof fetch;
  try {
    return await fn();
  } finally {
    globalThis.fetch = real;
  }
}

async function collect(adapter: AnthropicAdapter): Promise<LlmStreamEvent[]> {
  const out: LlmStreamEvent[] = [];
  for await (const ev of adapter.stream({
    messages: [{ role: "user", content: "hi" }],
    signal: new AbortController().signal,
  })) {
    out.push(ev);
  }
  return out;
}

test("Anthropic: text_delta frames become text events, end_turn → stop", async () => {
  const frames = [
    { type: "content_block_start", index: 0, content_block: { type: "text" } },
    { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "Hi" } },
    { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: " there" } },
    { type: "content_block_stop", index: 0 },
    { type: "message_delta", delta: { stop_reason: "end_turn" } },
    { type: "message_stop" },
  ];
  const events = await withMockedFetch(sseResponse(frames), () =>
    collect(new AnthropicAdapter({ apiKey: "k", model: "test" })),
  );
  const text = events
    .filter((e) => e.type === "text")
    .map((e) => (e as { delta: string }).delta)
    .join("");
  assert.equal(text, "Hi there");
  const done = events.at(-1) as { type: "done"; reason: string };
  assert.equal(done.reason, "stop");
});

test("Anthropic: tool_use blocks accumulate input_json_delta into a tool_call", async () => {
  const frames = [
    {
      type: "content_block_start",
      index: 0,
      content_block: { type: "tool_use", id: "toolu_1", name: "request_approval" },
    },
    {
      type: "content_block_delta",
      index: 0,
      delta: { type: "input_json_delta", partial_json: '{"question":"' },
    },
    {
      type: "content_block_delta",
      index: 0,
      delta: { type: "input_json_delta", partial_json: 'are you sure?"}' },
    },
    { type: "content_block_stop", index: 0 },
    { type: "message_delta", delta: { stop_reason: "tool_use" } },
  ];
  const events = await withMockedFetch(sseResponse(frames), () =>
    collect(new AnthropicAdapter({ apiKey: "k", model: "test" })),
  );
  const toolEvent = events.find((e) => e.type === "tool_call") as
    | { type: "tool_call"; call: { id: string; name: string; arguments: Record<string, unknown> } }
    | undefined;
  assert.ok(toolEvent);
  assert.equal(toolEvent.call.id, "toolu_1");
  assert.equal(toolEvent.call.name, "request_approval");
  assert.equal(toolEvent.call.arguments.question, "are you sure?");
  const done = events.at(-1) as { type: "done"; reason: string };
  assert.equal(done.reason, "tool_calls");
});

test("Anthropic: abnormal stream end reports done:error", async () => {
  const frames = [
    { type: "content_block_start", index: 0, content_block: { type: "text" } },
    { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "partial" } },
    // no message_delta with stop_reason
  ];
  const events = await withMockedFetch(sseResponse(frames), () =>
    collect(new AnthropicAdapter({ apiKey: "k", model: "test" })),
  );
  const done = events.at(-1) as { type: "done"; reason: string };
  assert.equal(done.reason, "error");
});

test("Anthropic: oversized tool args abort the stream", async () => {
  const big = "x".repeat(70_000);
  const frames = [
    {
      type: "content_block_start",
      index: 0,
      content_block: { type: "tool_use", id: "toolu_1", name: "f" },
    },
    {
      type: "content_block_delta",
      index: 0,
      delta: { type: "input_json_delta", partial_json: big },
    },
  ];
  await assert.rejects(
    withMockedFetch(sseResponse(frames), () =>
      collect(new AnthropicAdapter({ apiKey: "k", model: "test" })),
    ),
    /tool-call arguments exceeded/,
  );
});

test("Anthropic: HTTP error surfaces with status code", async () => {
  const err = new Response("overloaded", { status: 529 });
  await assert.rejects(
    withMockedFetch(err, () => collect(new AnthropicAdapter({ apiKey: "k", model: "test" }))),
    /\(529\)/,
  );
});
