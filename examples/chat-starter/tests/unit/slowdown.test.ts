import { test } from "node:test";
import assert from "node:assert/strict";
import { withSlowdown } from "../../src/lib/llm/slowdown.js";
import type { LlmAdapter, LlmStreamArgs, LlmStreamEvent } from "../../src/lib/llm/index.js";

function fixedAdapter(events: LlmStreamEvent[]): LlmAdapter {
  return {
    name: "fixed",
    async *stream(_args: LlmStreamArgs): AsyncGenerator<LlmStreamEvent, void, undefined> {
      for (const e of events) yield e;
    },
  };
}

async function consume(
  adapter: LlmAdapter,
  signal?: AbortSignal,
): Promise<{ events: LlmStreamEvent[]; elapsed: number }> {
  const start = Date.now();
  const out: LlmStreamEvent[] = [];
  for await (const ev of adapter.stream({
    messages: [{ role: "user", content: "x" }],
    signal: signal ?? new AbortController().signal,
  })) {
    out.push(ev);
  }
  return { events: out, elapsed: Date.now() - start };
}

test("withSlowdown returns the inner adapter unchanged when ms <= 0", () => {
  const inner = fixedAdapter([]);
  assert.equal(withSlowdown(inner, 0), inner);
  assert.equal(withSlowdown(inner, -5), inner);
  assert.equal(withSlowdown(inner, Number.NaN), inner);
});

test("withSlowdown sleeps between text events but not on tool_call / done", async () => {
  const events: LlmStreamEvent[] = [
    { type: "text", delta: "a" },
    { type: "text", delta: "b" },
    { type: "text", delta: "c" },
    { type: "done", reason: "stop" },
  ];
  const slow = withSlowdown(fixedAdapter(events), 30);
  const { events: out, elapsed } = await consume(slow);
  assert.equal(out.length, 4);
  // 3 text events × 30ms = ~90ms minimum.
  assert.ok(elapsed >= 80, `expected >=80ms elapsed, got ${elapsed}ms`);
});

test("withSlowdown wraps the adapter name for visibility in logs", () => {
  const inner = fixedAdapter([]);
  const slow = withSlowdown(inner, 50);
  assert.match(slow.name, /^fixed:slow\(50ms\)$/);
});

test("withSlowdown stops yielding when the signal is aborted mid-stream", async () => {
  const events: LlmStreamEvent[] = Array.from({ length: 50 }, (_, i) => ({
    type: "text" as const,
    delta: `${i}`,
  }));
  const controller = new AbortController();
  setTimeout(() => controller.abort(), 60);
  const slow = withSlowdown(fixedAdapter(events), 30);
  const { events: out } = await consume(slow, controller.signal);
  // With 30ms per text and abort at 60ms, we expect ~2 events through.
  assert.ok(out.length < 10, `expected aborted early, got ${out.length} events`);
});
