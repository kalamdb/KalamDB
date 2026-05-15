/* eslint-disable require-yield */
import { test } from "node:test";
import assert from "node:assert/strict";
import { withRetry } from "../../src/lib/llm/retry.js";
import type { LlmAdapter, LlmStreamArgs, LlmStreamEvent } from "../../src/lib/llm/index.js";

function makeAdapter(
  attemptHandlers: Array<() => AsyncGenerator<LlmStreamEvent, void, undefined>>,
): { adapter: LlmAdapter; attempts: number } {
  let attempts = 0;
  const adapter: LlmAdapter = {
    name: "mock",
    async *stream(_args: LlmStreamArgs): AsyncGenerator<LlmStreamEvent, void, undefined> {
      const handler = attemptHandlers[attempts++];
      if (!handler) throw new Error("no more attempts");
      for await (const ev of handler()) {
        yield ev;
      }
    },
  };
  return {
    adapter,
    get attempts() {
      return attempts;
    },
  };
}

async function* yields(events: LlmStreamEvent[]): AsyncGenerator<LlmStreamEvent, void, undefined> {
  for (const e of events) yield e;
}

async function* throws(err: Error): AsyncGenerator<LlmStreamEvent, void, undefined> {
  throw err;
}

test("retries on transient 5xx errors then succeeds", async () => {
  const a = makeAdapter([
    () => throws(new Error("OpenAI request failed (503): bad gateway")),
    () => throws(new Error("OpenAI request failed (502): bad gateway")),
    () =>
      yields([
        { type: "text", delta: "hello" },
        { type: "done", reason: "stop" },
      ]),
  ]);
  const wrapped = withRetry(a.adapter, { maxAttempts: 5, baseDelayMs: 1, maxDelayMs: 5 });
  const out: LlmStreamEvent[] = [];
  for await (const ev of wrapped.stream({
    messages: [{ role: "user", content: "x" }],
    signal: new AbortController().signal,
  })) {
    out.push(ev);
  }
  assert.equal(out.length, 2);
  assert.equal(a.attempts, 3);
});

test("retries on 429 then succeeds", async () => {
  const a = makeAdapter([
    () => throws(new Error("OpenAI request failed (429): too many requests")),
    () => yields([{ type: "done", reason: "stop" }]),
  ]);
  const wrapped = withRetry(a.adapter, { maxAttempts: 3, baseDelayMs: 1, maxDelayMs: 5 });
  for await (const _ev of wrapped.stream({
    messages: [{ role: "user", content: "x" }],
    signal: new AbortController().signal,
  })) {
    /* noop */
  }
  assert.equal(a.attempts, 2);
});

test("does NOT retry on 4xx client errors", async () => {
  const a = makeAdapter([
    () => throws(new Error("OpenAI request failed (400): bad input")),
    () => yields([{ type: "done", reason: "stop" }]),
  ]);
  const wrapped = withRetry(a.adapter, { maxAttempts: 5, baseDelayMs: 1, maxDelayMs: 5 });
  await assert.rejects(
    (async () => {
      for await (const _ev of wrapped.stream({
        messages: [{ role: "user", content: "x" }],
        signal: new AbortController().signal,
      })) {
        /* noop */
      }
    })(),
    /\(400\)/,
  );
  assert.equal(a.attempts, 1);
});

test("respects abort signal and does NOT retry", async () => {
  const controller = new AbortController();
  const a = makeAdapter([
    async function* () {
      controller.abort();
      throw Object.assign(new Error("aborted"), { name: "AbortError" });
    },
  ]);
  const wrapped = withRetry(a.adapter, { maxAttempts: 5, baseDelayMs: 1, maxDelayMs: 5 });
  await assert.rejects(
    (async () => {
      for await (const _ev of wrapped.stream({
        messages: [{ role: "user", content: "x" }],
        signal: controller.signal,
      })) {
        /* noop */
      }
    })(),
  );
  assert.equal(a.attempts, 1);
});

test("gives up after maxAttempts and rethrows the last error", async () => {
  const a = makeAdapter([
    () => throws(new Error("OpenAI request failed (503): one")),
    () => throws(new Error("OpenAI request failed (503): two")),
    () => throws(new Error("OpenAI request failed (503): three")),
  ]);
  const wrapped = withRetry(a.adapter, { maxAttempts: 3, baseDelayMs: 1, maxDelayMs: 5 });
  await assert.rejects(
    (async () => {
      for await (const _ev of wrapped.stream({
        messages: [{ role: "user", content: "x" }],
        signal: new AbortController().signal,
      })) {
        /* noop */
      }
    })(),
    /three/,
  );
  assert.equal(a.attempts, 3);
});

test("does NOT retry once events have started streaming", async () => {
  const a = makeAdapter([
    async function* () {
      yield { type: "text", delta: "hi" };
      throw new Error("OpenAI request failed (503): mid-stream");
    },
  ]);
  const wrapped = withRetry(a.adapter, { maxAttempts: 3, baseDelayMs: 1, maxDelayMs: 5 });
  await assert.rejects(
    (async () => {
      for await (const _ev of wrapped.stream({
        messages: [{ role: "user", content: "x" }],
        signal: new AbortController().signal,
      })) {
        /* noop */
      }
    })(),
    /mid-stream/,
  );
  assert.equal(a.attempts, 1);
});
