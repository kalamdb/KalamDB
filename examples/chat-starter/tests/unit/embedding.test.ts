import { test } from "node:test";
import assert from "node:assert/strict";
import {
  EMBEDDING_DIMENSIONS,
  embed,
  embeddingLiteral,
  fakeEmbed,
} from "../../src/lib/llm/embedding.js";

test("EMBEDDING_DIMENSIONS matches the chat.docs schema (384)", () => {
  assert.equal(EMBEDDING_DIMENSIONS, 384);
});

test("fakeEmbed returns a fixed-length unit-norm vector", () => {
  const v = fakeEmbed("hello world");
  assert.equal(v.length, 384);
  let norm = 0;
  for (const x of v) norm += x * x;
  assert.ok(Math.abs(norm - 1) < 1e-6, `expected unit norm, got ${norm}`);
});

test("fakeEmbed is deterministic — same input yields same output", () => {
  const a = fakeEmbed("topics and live queries");
  const b = fakeEmbed("topics and live queries");
  assert.deepEqual(a, b);
});

test("fakeEmbed gives different vectors for different inputs", () => {
  const a = fakeEmbed("topics");
  const b = fakeEmbed("approvals");
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff += Math.abs(a[i]! - b[i]!);
  assert.ok(diff > 0.5, `expected meaningfully different vectors, got total diff=${diff}`);
});

test("embed(): EMBEDDING_PROVIDER=fake forces fake mode even with an API key", async () => {
  const restoreKey = process.env.OPENAI_API_KEY;
  const restoreProv = process.env.EMBEDDING_PROVIDER;
  process.env.OPENAI_API_KEY = "sk-test";
  process.env.EMBEDDING_PROVIDER = "fake";
  try {
    const v = await embed("anything");
    assert.equal(v.length, 384);
  } finally {
    if (restoreKey === undefined) delete process.env.OPENAI_API_KEY;
    else process.env.OPENAI_API_KEY = restoreKey;
    if (restoreProv === undefined) delete process.env.EMBEDDING_PROVIDER;
    else process.env.EMBEDDING_PROVIDER = restoreProv;
  }
});

test("embed(): falls back to fake when no API key and no explicit provider", async () => {
  const restoreKey = process.env.OPENAI_API_KEY;
  const restoreProv = process.env.EMBEDDING_PROVIDER;
  delete process.env.OPENAI_API_KEY;
  delete process.env.EMBEDDING_PROVIDER;
  try {
    const v = await embed("hello");
    assert.equal(v.length, 384);
  } finally {
    if (restoreKey !== undefined) process.env.OPENAI_API_KEY = restoreKey;
    if (restoreProv !== undefined) process.env.EMBEDDING_PROVIDER = restoreProv;
  }
});

test("embed(): rejects empty input", async () => {
  await assert.rejects(embed(""), /non-empty string/);
  await assert.rejects(embed("   "), /non-empty string/);
});

test("embeddingLiteral renders a KalamDB-compatible vector literal", () => {
  const lit = embeddingLiteral([1, 0, -0.5]);
  assert.equal(lit, "[1.000000,0.000000,-0.500000]");
});
