import { test } from "node:test";
import assert from "node:assert/strict";
import { OpenAiAdapter } from "../../src/lib/llm/openai.js";
import type { LlmStreamEvent } from "../../src/lib/llm/index.js";

// Build a Response whose body is the given SSE frames, terminated by [DONE].
function sseResponse(frames: string[], status = 200): Response {
  const encoder = new TextEncoder();
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const frame of frames) {
        controller.enqueue(encoder.encode(`data: ${frame}\n\n`));
      }
      controller.enqueue(encoder.encode("data: [DONE]\n\n"));
      controller.close();
    },
  });
  return new Response(body, {
    status,
    headers: { "content-type": "text/event-stream" },
  });
}

async function withMockedFetch<T>(
  responses: Response | Response[] | ((req: Request) => Response | Promise<Response>),
  fn: () => Promise<T>,
): Promise<T> {
  const real = globalThis.fetch;
  let i = 0;
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    const req = new Request(input, init);
    if (typeof responses === "function") return responses(req);
    if (Array.isArray(responses)) return responses[i++ % responses.length]!;
    return responses;
  }) as typeof fetch;
  try {
    return await fn();
  } finally {
    globalThis.fetch = real;
  }
}

async function collect(adapter: OpenAiAdapter): Promise<LlmStreamEvent[]> {
  const out: LlmStreamEvent[] = [];
  for await (const ev of adapter.stream({
    messages: [{ role: "user", content: "hi" }],
    signal: new AbortController().signal,
  })) {
    out.push(ev);
  }
  return out;
}

test("OpenAI: streams text deltas and finishes with stop", async () => {
  const frames = [
    JSON.stringify({ choices: [{ delta: { content: "Hello" } }] }),
    JSON.stringify({ choices: [{ delta: { content: " world" } }] }),
    JSON.stringify({ choices: [{ finish_reason: "stop" }] }),
  ];
  const events = await withMockedFetch(sseResponse(frames), () =>
    collect(new OpenAiAdapter({ apiKey: "k", model: "test" })),
  );
  const text = events
    .filter((e) => e.type === "text")
    .map((e) => (e as { delta: string }).delta)
    .join("");
  assert.equal(text, "Hello world");
  const done = events.at(-1) as { type: "done"; reason: string };
  assert.equal(done.type, "done");
  assert.equal(done.reason, "stop");
});

test("OpenAI: assembles a tool_call across multiple argument deltas", async () => {
  const frames = [
    JSON.stringify({
      choices: [
        {
          delta: {
            tool_calls: [{ index: 0, id: "call_1", function: { name: "request_approval" } }],
          },
        },
      ],
    }),
    JSON.stringify({
      choices: [
        { delta: { tool_calls: [{ index: 0, function: { arguments: '{"question":"' } }] } },
      ],
    }),
    JSON.stringify({
      choices: [
        { delta: { tool_calls: [{ index: 0, function: { arguments: 'are you sure?"}' } }] } },
      ],
    }),
    JSON.stringify({ choices: [{ finish_reason: "tool_calls" }] }),
  ];
  const events = await withMockedFetch(sseResponse(frames), () =>
    collect(new OpenAiAdapter({ apiKey: "k", model: "test" })),
  );
  const toolEvent = events.find((e) => e.type === "tool_call") as
    | { type: "tool_call"; call: { id: string; name: string; arguments: Record<string, unknown> } }
    | undefined;
  assert.ok(toolEvent);
  assert.equal(toolEvent.call.id, "call_1");
  assert.equal(toolEvent.call.name, "request_approval");
  assert.equal(toolEvent.call.arguments.question, "are you sure?");
  const done = events.at(-1) as { type: "done"; reason: string };
  assert.equal(done.reason, "tool_calls");
});

test("OpenAI: content_filter finish_reason is mapped to done:error", async () => {
  const frames = [
    JSON.stringify({ choices: [{ delta: { content: "blocked" } }] }),
    JSON.stringify({ choices: [{ finish_reason: "content_filter" }] }),
  ];
  const events = await withMockedFetch(sseResponse(frames), () =>
    collect(new OpenAiAdapter({ apiKey: "k", model: "test" })),
  );
  const done = events.at(-1) as { type: "done"; reason: string };
  assert.equal(done.reason, "error");
});

test("OpenAI: abnormal stream end (no finish_reason) reports done:error", async () => {
  const frames = [
    JSON.stringify({ choices: [{ delta: { content: "partial" } }] }),
    // no finish_reason frame
  ];
  const events = await withMockedFetch(sseResponse(frames), () =>
    collect(new OpenAiAdapter({ apiKey: "k", model: "test" })),
  );
  const done = events.at(-1) as { type: "done"; reason: string };
  assert.equal(done.reason, "error");
});

test("OpenAI: oversized tool args abort the stream", async () => {
  const big = "x".repeat(70_000);
  const frames = [
    JSON.stringify({
      choices: [{ delta: { tool_calls: [{ index: 0, id: "call_1", function: { name: "f" } }] } }],
    }),
    JSON.stringify({
      choices: [{ delta: { tool_calls: [{ index: 0, function: { arguments: big } }] } }],
    }),
  ];
  await assert.rejects(
    withMockedFetch(sseResponse(frames), () =>
      collect(new OpenAiAdapter({ apiKey: "k", model: "test" })),
    ),
    /tool-call arguments exceeded/,
  );
});

test("OpenAI: HTTP error surfaces as a throw including status code", async () => {
  const errResponse = new Response("rate limit", { status: 429 });
  await assert.rejects(
    withMockedFetch(errResponse, () => collect(new OpenAiAdapter({ apiKey: "k", model: "test" }))),
    /\(429\)/,
  );
});
